#[cfg(test)]
#[path = "app_runtime_queue/tests.rs"]
mod tests;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use syncplay_client_app::app_boundary::commands::LocalOffsetCommand;

use super::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiPendingCompletionRequest,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use super::shell_state::{GuiShellAction, SyncplayGuiShellAppState};
use super::support::normalized_editable_text;

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(super) struct GuiQueuedRuntimeBridgeHandle {
    queued_actions: Arc<Mutex<VecDeque<GuiShellAction>>>,
    queued_requests: Arc<Mutex<VecDeque<GuiRuntimeRequest>>>,
}

#[allow(dead_code)]
impl GuiQueuedRuntimeBridgeHandle {
    pub(super) fn push_action(&self, action: GuiShellAction) {
        self.push_actions([action]);
    }

    pub(super) fn push_actions<I>(&self, actions: I)
    where
        I: IntoIterator<Item = GuiShellAction>,
    {
        let mut queue = self
            .queued_actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(actions);
    }

    pub(super) fn drain_actions(&self) -> Vec<GuiShellAction> {
        let mut queue = self
            .queued_actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(super) fn push_request(&self, request: GuiRuntimeRequest) {
        self.push_requests([request]);
    }

    pub(super) fn push_requests<I>(&self, requests: I)
    where
        I: IntoIterator<Item = GuiRuntimeRequest>,
    {
        let mut queue = self
            .queued_requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(requests);
    }

    pub(super) fn drain_requests(&self) -> Vec<GuiRuntimeRequest> {
        let mut queue = self
            .queued_requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(super) fn drain_preview_response_actions(&self) -> Vec<GuiShellAction> {
        self.drain_requests()
            .into_iter()
            .flat_map(|request| request.preview_actions())
            .collect()
    }
}

#[allow(dead_code)]
#[derive(Default)]
pub(super) struct GuiQueuedRuntimeBridge {
    handle: GuiQueuedRuntimeBridgeHandle,
    show_manual_pending_controls: bool,
}

#[allow(dead_code)]
impl GuiQueuedRuntimeBridge {
    pub(super) fn new() -> (Self, GuiQueuedRuntimeBridgeHandle) {
        Self::new_with_manual_pending_controls(false)
    }

    pub(super) fn new_with_manual_pending_controls(
        show_manual_pending_controls: bool,
    ) -> (Self, GuiQueuedRuntimeBridgeHandle) {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        (
            Self {
                handle: handle.clone(),
                show_manual_pending_controls,
            },
            handle,
        )
    }
}

impl GuiNativeRuntimeBridge for GuiQueuedRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool {
        self.show_manual_pending_controls
    }

    fn drain_runtime_actions(&mut self) -> Vec<GuiShellAction> {
        self.handle.drain_actions()
    }

    fn dispatch_runtime_request(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        self.handle.push_request(request);
        Vec::new()
    }

    fn actions_for_open_media_files(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction> {
        if !paths.is_empty() {
            self.handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
            });
        }
        Vec::new()
    }

    fn actions_for_seek_offset(&mut self, offset_seconds: f64) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SeekOffset(offset_seconds));
        Vec::new()
    }

    fn actions_for_undo_seek(&mut self) -> Vec<GuiShellAction> {
        self.handle.push_request(GuiRuntimeRequest::UndoSeek);
        Vec::new()
    }

    fn actions_for_set_offset(&mut self, command: LocalOffsetCommand) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetOffset(command));
        Vec::new()
    }

    fn actions_for_autoplay_enabled_change(&mut self, enabled: bool) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetAutoplayEnabled(enabled));
        Vec::new()
    }

    fn actions_for_autoplay_threshold_change(&mut self, threshold: usize) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetAutoplayThreshold(threshold));
        Vec::new()
    }

    fn actions_for_main_window_user_media_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        if normalized_editable_text(&target).is_some() {
            self.handle
                .push_request(GuiRuntimeRequest::OpenMainWindowUserMedia(target));
        }
        Vec::new()
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        if normalized_editable_text(&target).is_some() {
            self.handle
                .push_request(GuiRuntimeRequest::OpenMainWindowUserContainingFolder(
                    target,
                ));
        }
        Vec::new()
    }

    fn actions_for_room_join(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        room: String,
    ) -> Vec<GuiShellAction> {
        if let Some(room) = normalized_editable_text(&room) {
            self.handle.push_request(GuiRuntimeRequest::SetRoom(room));
        }
        Vec::new()
    }

    fn actions_for_room_leave(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ReturnToDefaultRoom);
        Vec::new()
    }

    fn actions_for_local_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        ready: bool,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetLocalReady(ready));
        Vec::new()
    }

    fn actions_for_main_window_user_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        username: String,
        ready: bool,
    ) -> Vec<GuiShellAction> {
        if normalized_editable_text(&username).is_some() {
            self.handle
                .push_request(GuiRuntimeRequest::SetReadyForUser { username, ready });
        }
        Vec::new()
    }

    fn actions_for_controller_auth_request(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        room: String,
        password: String,
    ) -> Vec<GuiShellAction> {
        if normalized_editable_text(&room).is_some()
            && normalized_editable_text(&password).is_some()
        {
            self.handle
                .push_request(GuiRuntimeRequest::RequestControllerAuth { room, password });
        }
        Vec::new()
    }

    fn actions_for_playlist_entry_commit(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        entry: String,
        select_after_queue: bool,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::QueuePlaylistEntry {
                entry,
                select_after_queue,
            });
        Vec::new()
    }

    fn actions_for_playlist_selection_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        index: usize,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetPlaylistIndex(index));
        Vec::new()
    }

    fn actions_for_playlist_entry_removal(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        index: usize,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::DeletePlaylistIndex(index));
        Vec::new()
    }

    fn actions_for_playlist_reorder(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        playlist: Vec<String>,
        selected_index: Option<usize>,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ReplacePlaylist {
                files: playlist,
                selected_index,
            });
        Vec::new()
    }

    fn actions_for_playlist_undo(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::UndoPlaylistChange);
        Vec::new()
    }

    fn actions_for_playlist_shuffle_remaining(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ShuffleRemainingPlaylist);
        Vec::new()
    }

    fn actions_for_playlist_shuffle_entire(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ShuffleEntirePlaylist);
        Vec::new()
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        if let Some(request) = GuiPendingCompletionRequest::from_state(state) {
            self.handle
                .push_request(GuiRuntimeRequest::CompletePendingOperation(request));
        }
        Vec::new()
    }

    fn actions_for_pending_cancel(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        if let Some(pending) = state.pending_operation.as_ref() {
            self.handle
                .push_request(GuiRuntimeRequest::CancelPendingOperation(pending.kind));
        }
        Vec::new()
    }
}

pub(super) struct GuiQueuedRuntimeOwnerPump<TOwner> {
    handle: GuiQueuedRuntimeBridgeHandle,
    owner: TOwner,
}

impl<TOwner> GuiQueuedRuntimeOwnerPump<TOwner> {
    pub(super) fn new(handle: GuiQueuedRuntimeBridgeHandle, owner: TOwner) -> Self {
        Self { handle, owner }
    }
}

impl<TOwner> GuiNativeRuntimePump for GuiQueuedRuntimeOwnerPump<TOwner>
where
    TOwner: GuiQueuedRuntimeOwner,
{
    fn pump(&mut self, state: &SyncplayGuiShellAppState) {
        self.owner.pump(&self.handle, state);
    }
}
