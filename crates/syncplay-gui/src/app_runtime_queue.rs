#[cfg(test)]
#[path = "app_runtime_queue/tests.rs"]
mod tests;

use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, Weak},
    thread::{self, JoinHandle},
    time::Duration,
};

use syncplay_client_app::app_boundary::commands::LocalOffsetCommand;

use super::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiPendingCompletionRequest,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use super::shell_state::{GuiShellAction, SyncplayGuiShellAppState};
use super::support::{nonempty_room_name_text, normalized_editable_text};

type GuiRepaintNotifier = Arc<dyn Fn() + Send + Sync>;

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(super) struct GuiQueuedRuntimeBridgeHandle {
    queued_actions: Arc<Mutex<VecDeque<GuiShellAction>>>,
    queued_requests: Arc<Mutex<VecDeque<GuiRuntimeRequest>>>,
    repaint_notifier: Arc<Mutex<Option<GuiRepaintNotifier>>>,
    threaded_runtime_owner: Arc<Mutex<Option<Weak<GuiThreadedRuntimeOwnerShared>>>>,
}

#[allow(dead_code)]
impl GuiQueuedRuntimeBridgeHandle {
    pub(super) fn set_repaint_notifier<F>(&self, notifier: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut repaint_notifier = self
            .repaint_notifier
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *repaint_notifier = Some(Arc::new(notifier));
    }

    pub(super) fn clear_repaint_notifier(&self) {
        let mut repaint_notifier = self
            .repaint_notifier
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *repaint_notifier = None;
    }

    fn set_threaded_runtime_owner(&self, owner: &Arc<GuiThreadedRuntimeOwnerShared>) {
        let mut threaded_runtime_owner = self
            .threaded_runtime_owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *threaded_runtime_owner = Some(Arc::downgrade(owner));
    }

    fn notify_threaded_runtime_owner(&self) {
        let shared = self
            .threaded_runtime_owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(Weak::upgrade);
        let Some(shared) = shared else {
            return;
        };
        {
            let mut shared_state = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            shared_state.runtime_wake_revision = shared_state.runtime_wake_revision.wrapping_add(1);
        }
        shared.wake.notify_one();
    }

    fn notify_repaint(&self) {
        let repaint_notifier = self
            .repaint_notifier
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(repaint_notifier) = repaint_notifier else {
            return;
        };
        repaint_notifier();
    }

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
        let previous_len = queue.len();
        queue.extend(actions);
        let queued_actions = queue.len().saturating_sub(previous_len);
        drop(queue);
        if queued_actions != 0 {
            self.notify_repaint();
        }
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
        let previous_len = queue.len();
        queue.extend(requests);
        let queued_requests = queue.len().saturating_sub(previous_len);
        drop(queue);
        if queued_requests != 0 {
            self.notify_threaded_runtime_owner();
        }
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
                playlist_insert_slot: None,
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
        if let Some(room) = nonempty_room_name_text(&room) {
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
        if nonempty_room_name_text(&room).is_some()
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

#[derive(Default)]
struct GuiThreadedRuntimeOwnerSharedState {
    latest_state: Option<Arc<SyncplayGuiShellAppState>>,
    latest_state_revision: u64,
    runtime_wake_revision: u64,
    stop_requested: bool,
}

#[derive(Default)]
struct GuiThreadedRuntimeOwnerShared {
    state: Mutex<GuiThreadedRuntimeOwnerSharedState>,
    wake: Condvar,
}

pub(super) struct GuiThreadedRuntimeOwnerPump {
    last_submitted_state: Option<Arc<SyncplayGuiShellAppState>>,
    shared: Arc<GuiThreadedRuntimeOwnerShared>,
    worker: Option<JoinHandle<()>>,
}

impl GuiThreadedRuntimeOwnerPump {
    const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

    pub(super) fn new<TOwner>(handle: GuiQueuedRuntimeBridgeHandle, owner: TOwner) -> Self
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        Self::new_with_poll_interval(handle, owner, Self::DEFAULT_POLL_INTERVAL)
    }

    pub(super) fn new_with_poll_interval<TOwner>(
        handle: GuiQueuedRuntimeBridgeHandle,
        owner: TOwner,
        poll_interval: Duration,
    ) -> Self
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        let shared = Arc::new(GuiThreadedRuntimeOwnerShared::default());
        handle.set_threaded_runtime_owner(&shared);
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("syncplay-gui-runtime".to_owned())
            .spawn(move || {
                Self::run_worker_loop(handle, owner, worker_shared, poll_interval);
            })
            .expect("failed to spawn syncplay GUI runtime thread");
        Self {
            last_submitted_state: None,
            shared,
            worker: Some(worker),
        }
    }

    fn run_worker_loop<TOwner>(
        handle: GuiQueuedRuntimeBridgeHandle,
        mut owner: TOwner,
        shared: Arc<GuiThreadedRuntimeOwnerShared>,
        poll_interval: Duration,
    ) where
        TOwner: GuiQueuedRuntimeOwner,
    {
        let mut latest_state = None;
        let mut latest_revision = 0_u64;
        let mut latest_runtime_wake_revision = 0_u64;

        loop {
            let mut timed_out = false;
            let mut shared_state = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            loop {
                if shared_state.stop_requested {
                    return;
                }
                if shared_state.latest_state_revision != latest_revision {
                    latest_revision = shared_state.latest_state_revision;
                    latest_state = shared_state.latest_state.clone();
                }
                if shared_state.runtime_wake_revision != latest_runtime_wake_revision || timed_out {
                    latest_runtime_wake_revision = shared_state.runtime_wake_revision;
                    break;
                }
                if latest_state.is_some() {
                    let (next_shared_state, timeout) = shared
                        .wake
                        .wait_timeout(shared_state, poll_interval)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    shared_state = next_shared_state;
                    timed_out = timeout.timed_out();
                    continue;
                }
                shared_state = shared
                    .wake
                    .wait(shared_state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }

            drop(shared_state);

            if let Some(state) = latest_state.as_ref() {
                owner.pump(&handle, state);
            }
        }
    }
}

impl GuiNativeRuntimePump for GuiThreadedRuntimeOwnerPump {
    fn pump(&mut self, state: &SyncplayGuiShellAppState) {
        let state_changed = self.last_submitted_state.as_deref() != Some(state);
        let mut shared_state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state_changed {
            let snapshot = Arc::new(state.clone());
            self.last_submitted_state = Some(snapshot.clone());
            shared_state.latest_state = Some(snapshot);
            shared_state.latest_state_revision = shared_state.latest_state_revision.wrapping_add(1);
        }
        shared_state.runtime_wake_revision = shared_state.runtime_wake_revision.wrapping_add(1);
        drop(shared_state);
        self.shared.wake.notify_one();
    }
}

impl Drop for GuiThreadedRuntimeOwnerPump {
    fn drop(&mut self) {
        {
            let mut shared_state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            shared_state.stop_requested = true;
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("syncplay-gui runtime thread panicked during shutdown");
        }
    }
}
