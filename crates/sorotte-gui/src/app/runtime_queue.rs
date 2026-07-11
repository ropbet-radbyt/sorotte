#[cfg(test)]
mod tests;

use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, Weak},
    thread::{self, JoinHandle},
    time::Duration,
};

use sorotte_client_app::app_boundary::commands::LocalOffsetCommand;

use super::feature_slices::{GuiClientCommand, GuiRuntimeInput};
use super::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiPendingCompletionRequest,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use super::shell_state::{
    GuiPendingOperationKind, GuiShellAction, GuiTransientNotificationLevel, SorotteGuiShellAppState,
};
use super::support::{nonempty_room_name_text, normalized_editable_text};

type GuiRepaintNotifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub(super) struct GuiQueuedRuntimeBridgeHandle {
    queued_actions: Arc<Mutex<VecDeque<GuiShellAction>>>,
    queued_commands: Arc<Mutex<VecDeque<GuiClientCommand>>>,
    repaint_notifier: Arc<Mutex<Option<GuiRepaintNotifier>>>,
    threaded_runtime_owner: Arc<Mutex<Option<Weak<GuiThreadedRuntimeOwnerShared>>>>,
}

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
            .queued_commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_len = queue.len();
        queue.extend(
            requests
                .into_iter()
                .map(GuiClientCommand::from_compatibility_request),
        );
        let queued_commands = queue.len().saturating_sub(previous_len);
        drop(queue);
        if queued_commands != 0 {
            self.notify_threaded_runtime_owner();
        }
    }

    pub(super) fn drain_requests(&self) -> Vec<GuiRuntimeRequest> {
        self.drain_client_commands()
            .into_iter()
            .map(GuiClientCommand::into_compatibility_request)
            .collect()
    }

    pub(super) fn drain_client_commands(&self) -> Vec<GuiClientCommand> {
        let mut queue = self
            .queued_commands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    #[cfg(test)]
    pub(super) fn drain_preview_response_actions(&self) -> Vec<GuiShellAction> {
        self.drain_requests()
            .into_iter()
            .flat_map(|request| request.preview_actions())
            .collect()
    }
}

#[derive(Default)]
pub(super) struct GuiQueuedRuntimeBridge {
    handle: GuiQueuedRuntimeBridgeHandle,
    show_manual_pending_controls: bool,
    queued_pending_completion: Option<GuiPendingOperationKind>,
}

impl GuiQueuedRuntimeBridge {
    #[cfg(test)]
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
                queued_pending_completion: None,
            },
            handle,
        )
    }

    fn queue_pending_completion_if_needed(&mut self, state: &SorotteGuiShellAppState) {
        let pending_kind = state.pending_operation.as_ref().map(|pending| pending.kind);
        let Some(request) = GuiPendingCompletionRequest::from_state(state) else {
            self.queued_pending_completion = None;
            return;
        };
        if self.queued_pending_completion == pending_kind
            && pending_kind != Some(GuiPendingOperationKind::SearchMissingMedia)
        {
            return;
        }
        self.handle
            .push_request(GuiRuntimeRequest::CompletePendingOperation(request));
        self.queued_pending_completion = pending_kind;
    }

    fn runtime_action_clears_pending_operation(action: &GuiShellAction) -> bool {
        matches!(
            action,
            GuiShellAction::CompleteConfigurationSave(_)
                | GuiShellAction::CancelConfigurationSave
                | GuiShellAction::CompleteConfigurationReset(_)
                | GuiShellAction::CancelConfigurationReset
                | GuiShellAction::CompleteConfigurationReload(_)
                | GuiShellAction::CancelConfigurationReload
                | GuiShellAction::CompleteClearGuiData
                | GuiShellAction::CancelClearGuiData
                | GuiShellAction::CompleteConfigStorageRootChange { .. }
                | GuiShellAction::CancelConfigStorageRootChange
                | GuiShellAction::CompletePendingOperation
                | GuiShellAction::CancelPendingOperation
                | GuiShellAction::CompleteSavedServerConnect
                | GuiShellAction::CancelSavedServerConnect
                | GuiShellAction::CompleteSessionDisconnect
                | GuiShellAction::CancelSessionDisconnect
                | GuiShellAction::CompleteSelectedPublicServerConnect
                | GuiShellAction::CompletePublicServerRefresh(_)
                | GuiShellAction::CompleteMissingMediaSearch(_)
                | GuiShellAction::CompletePlaybackPauseState(_)
                | GuiShellAction::CancelPlaybackPauseState
                | GuiShellAction::CompletePlaybackPauseToggle
                | GuiShellAction::CancelPlaybackPauseToggle
                | GuiShellAction::CompleteLocalChatSend
        )
    }

    fn runtime_action_refreshes_pending_completion(action: &GuiShellAction) -> bool {
        matches!(action, GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(_))
    }
}

impl GuiNativeRuntimeBridge for GuiQueuedRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool {
        self.show_manual_pending_controls
    }

    fn drain_runtime_actions(&mut self) -> Vec<GuiShellAction> {
        let actions = self.handle.drain_actions();
        let search_pending_should_retry = self.queued_pending_completion
            == Some(GuiPendingOperationKind::SearchMissingMedia)
            && actions
                .iter()
                .any(Self::runtime_action_refreshes_pending_completion);
        if actions
            .iter()
            .any(Self::runtime_action_clears_pending_operation)
            || search_pending_should_retry
        {
            self.queued_pending_completion = None;
        }
        actions
    }

    fn dispatch_runtime_request(
        &mut self,
        _state: &SorotteGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        self.handle.push_request(request);
        Vec::new()
    }

    fn actions_for_open_media_files(
        &mut self,
        _state: &SorotteGuiShellAppState,
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
        _state: &SorotteGuiShellAppState,
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
        _state: &SorotteGuiShellAppState,
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
        _state: &SorotteGuiShellAppState,
        room: String,
    ) -> Vec<GuiShellAction> {
        if let Some(room) = nonempty_room_name_text(&room) {
            self.handle.push_request(GuiRuntimeRequest::SetRoom(room));
        }
        Vec::new()
    }

    fn actions_for_room_leave(&mut self, _state: &SorotteGuiShellAppState) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ReturnToDefaultRoom);
        Vec::new()
    }

    fn actions_for_local_readiness_change(
        &mut self,
        _state: &SorotteGuiShellAppState,
        ready: bool,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetLocalReady(ready));
        Vec::new()
    }

    fn actions_for_main_window_user_readiness_change(
        &mut self,
        _state: &SorotteGuiShellAppState,
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
        _state: &SorotteGuiShellAppState,
        room: String,
        password: String,
    ) -> Vec<GuiShellAction> {
        if nonempty_room_name_text(&room).is_some() && normalized_editable_text(&password).is_some()
        {
            self.handle
                .push_request(GuiRuntimeRequest::RequestControllerAuth {
                    room,
                    password: password.into(),
                });
        }
        Vec::new()
    }

    fn actions_for_playlist_entry_commit(
        &mut self,
        _state: &SorotteGuiShellAppState,
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

    fn actions_for_playlist_activation(
        &mut self,
        _state: &SorotteGuiShellAppState,
        index: usize,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::SetPlaylistIndex(index));
        Vec::new()
    }

    fn actions_for_playlist_entry_removal(
        &mut self,
        _state: &SorotteGuiShellAppState,
        index: usize,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::DeletePlaylistIndex(index));
        Vec::new()
    }

    fn actions_for_playlist_reorder(
        &mut self,
        _state: &SorotteGuiShellAppState,
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
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::UndoPlaylistChange);
        Vec::new()
    }

    fn actions_for_playlist_shuffle_remaining(
        &mut self,
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ShuffleRemainingPlaylist);
        Vec::new()
    }

    fn actions_for_playlist_shuffle_entire(
        &mut self,
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.handle
            .push_request(GuiRuntimeRequest::ShuffleEntirePlaylist);
        Vec::new()
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        self.queue_pending_completion_if_needed(state);
        Vec::new()
    }

    fn actions_for_pending_cancel(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        if let Some(pending) = state.pending_operation.as_ref() {
            self.handle
                .push_request(GuiRuntimeRequest::CancelPendingOperation(pending.kind));
        }
        Vec::new()
    }
}

#[cfg(test)]
pub(super) struct GuiQueuedRuntimeOwnerPump<TOwner> {
    handle: GuiQueuedRuntimeBridgeHandle,
    owner: TOwner,
    last_input: Option<GuiRuntimeInput>,
}

#[cfg(test)]
impl<TOwner> GuiQueuedRuntimeOwnerPump<TOwner> {
    pub(super) fn new(handle: GuiQueuedRuntimeBridgeHandle, owner: TOwner) -> Self {
        Self {
            handle,
            owner,
            last_input: None,
        }
    }
}

#[cfg(test)]
impl<TOwner> GuiNativeRuntimePump for GuiQueuedRuntimeOwnerPump<TOwner>
where
    TOwner: GuiQueuedRuntimeOwner,
{
    fn pump(&mut self, state: &SorotteGuiShellAppState) {
        if !self
            .last_input
            .as_ref()
            .is_some_and(|input| input.matches_shell(state))
        {
            let input = GuiRuntimeInput::from_shell(state);
            self.owner.input_changed(&self.handle, &input);
            self.last_input = Some(input);
        }
        self.owner.poll(&self.handle);
    }
}

#[derive(Default)]
struct GuiThreadedRuntimeOwnerSharedState {
    latest_input: Option<Arc<GuiRuntimeInput>>,
    latest_input_revision: u64,
    runtime_wake_revision: u64,
    stop_requested: bool,
}

#[derive(Default)]
struct GuiThreadedRuntimeOwnerShared {
    state: Mutex<GuiThreadedRuntimeOwnerSharedState>,
    wake: Condvar,
}

pub(super) struct GuiThreadedRuntimeOwnerPump {
    last_submitted_input: Option<Arc<GuiRuntimeInput>>,
    shared: Arc<GuiThreadedRuntimeOwnerShared>,
    worker: Option<JoinHandle<()>>,
}

impl GuiThreadedRuntimeOwnerPump {
    const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

    pub(super) fn new<TOwner>(
        handle: GuiQueuedRuntimeBridgeHandle,
        owner: TOwner,
    ) -> Result<Self, String>
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        Self::new_with_poll_interval(handle, owner, Self::DEFAULT_POLL_INTERVAL)
    }

    pub(super) fn new_with_poll_interval<TOwner>(
        handle: GuiQueuedRuntimeBridgeHandle,
        owner: TOwner,
        poll_interval: Duration,
    ) -> Result<Self, String>
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        let shared = Arc::new(GuiThreadedRuntimeOwnerShared::default());
        let worker_shared = shared.clone();
        let worker_handle = handle.clone();
        let worker = thread::Builder::new()
            .name("sorotte-gui-runtime".to_owned())
            .spawn(move || {
                Self::run_worker_loop(worker_handle, owner, worker_shared, poll_interval);
            })
            .map_err(|error| format!("failed to spawn syncplay GUI runtime thread: {error}"))?;
        handle.set_threaded_runtime_owner(&shared);
        Ok(Self {
            last_submitted_input: None,
            shared,
            worker: Some(worker),
        })
    }

    fn run_worker_loop<TOwner>(
        handle: GuiQueuedRuntimeBridgeHandle,
        mut owner: TOwner,
        shared: Arc<GuiThreadedRuntimeOwnerShared>,
        poll_interval: Duration,
    ) where
        TOwner: GuiQueuedRuntimeOwner,
    {
        let mut latest_input = None;
        let mut latest_revision = 0_u64;
        let mut latest_runtime_wake_revision = 0_u64;

        loop {
            let mut timed_out = false;
            let mut changed_input = None;
            let mut shared_state = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            loop {
                if shared_state.stop_requested {
                    return;
                }
                if shared_state.latest_input_revision != latest_revision {
                    latest_revision = shared_state.latest_input_revision;
                    latest_input = shared_state.latest_input.clone();
                    changed_input = latest_input.clone();
                }
                if shared_state.runtime_wake_revision != latest_runtime_wake_revision
                    || timed_out
                    || changed_input.is_some()
                {
                    latest_runtime_wake_revision = shared_state.runtime_wake_revision;
                    break;
                }
                if latest_input.is_some() {
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

            if let Some(input) = changed_input.as_ref() {
                owner.input_changed(&handle, input);
            }
            if latest_input.is_some() {
                owner.poll(&handle);
            }
        }
    }
}

pub(super) struct GuiRuntimeThreadUnavailablePump {
    handle: GuiQueuedRuntimeBridgeHandle,
    startup_error: String,
    startup_reported: bool,
}

impl GuiRuntimeThreadUnavailablePump {
    pub(super) fn new(handle: GuiQueuedRuntimeBridgeHandle, startup_error: String) -> Self {
        Self {
            handle,
            startup_error,
            startup_reported: false,
        }
    }

    fn startup_error_actions(error: &str) -> Vec<GuiShellAction> {
        let message = format!(
            "Sorotte GUI runtime could not start: {error}. Runtime actions are disabled until Sorotte is restarted."
        );
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn ignored_requests_message(request_count: usize) -> String {
        match request_count {
            1 => "Ignored 1 runtime request because the Sorotte GUI runtime is unavailable."
                .to_owned(),
            count => format!(
                "Ignored {count} runtime requests because the Sorotte GUI runtime is unavailable."
            ),
        }
    }
}

impl GuiNativeRuntimePump for GuiRuntimeThreadUnavailablePump {
    fn pump(&mut self, _state: &SorotteGuiShellAppState) {
        let mut actions = Vec::new();
        if !self.startup_reported {
            actions.extend(Self::startup_error_actions(&self.startup_error));
            self.startup_reported = true;
        }
        let ignored_requests = self.handle.drain_requests().len();
        if ignored_requests != 0 {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: Self::ignored_requests_message(ignored_requests),
            });
        }
        self.handle.push_actions(actions);
    }
}

impl GuiNativeRuntimePump for GuiThreadedRuntimeOwnerPump {
    fn pump(&mut self, state: &SorotteGuiShellAppState) {
        if self
            .last_submitted_input
            .as_deref()
            .is_some_and(|input| input.matches_shell(state))
        {
            return;
        }
        let input = GuiRuntimeInput::from_shell(state);
        let mut shared_state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let snapshot = Arc::new(input);
        self.last_submitted_input = Some(snapshot.clone());
        shared_state.latest_input = Some(snapshot);
        shared_state.latest_input_revision = shared_state.latest_input_revision.wrapping_add(1);
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
            eprintln!("sorotte-gui runtime thread panicked during shutdown");
        }
    }
}
