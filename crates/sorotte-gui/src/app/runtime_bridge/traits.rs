use super::*;
use crate::app::feature_slices::GuiRuntimeInput;

pub(in crate::app) trait GuiNativeRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool;

    fn drain_runtime_actions(&mut self) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn dispatch_runtime_request(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _request: GuiRuntimeRequest,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_open_media_files(
        &mut self,
        state: &SorotteGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction>;

    fn actions_for_selected_media_files(
        &mut self,
        state: &SorotteGuiShellAppState,
        paths: Vec<String>,
    ) -> Vec<GuiShellAction> {
        self.actions_for_open_media_files(
            state,
            paths,
            state.playlist_backed_media_opens_preferred(),
        )
    }

    fn actions_for_dropped_files(
        &mut self,
        state: &SorotteGuiShellAppState,
        request: GuiDroppedFilesRequest,
    ) -> Vec<GuiShellAction> {
        self.dispatch_runtime_request(
            state,
            GuiRuntimeRequest::OpenMediaFiles {
                paths: request.paths,
                load_into_shared_playlist: request.target.load_into_shared_playlist(state),
                playlist_insert_slot: request.playlist_insert_slot,
            },
        )
    }

    fn actions_for_seek_offset(&mut self, offset_seconds: f64) -> Vec<GuiShellAction>;

    fn actions_for_undo_seek(&mut self) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_set_offset(&mut self, _command: LocalOffsetCommand) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_autoplay_enabled_change(&mut self, _enabled: bool) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_autoplay_threshold_change(&mut self, _threshold: usize) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_media_open(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_join(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _room: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_leave(&mut self, _state: &SorotteGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_local_readiness_change(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_readiness_change(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _username: String,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_controller_auth_request(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _room: String,
        _password: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_commit(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _entry: String,
        _select_after_queue: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_activation(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_removal(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_reorder(
        &mut self,
        _state: &SorotteGuiShellAppState,
        _playlist: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_undo(
        &mut self,
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_remaining(
        &mut self,
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_entire(
        &mut self,
        _state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction>;

    fn actions_for_pending_cancel(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction>;
}

pub(in crate::app) trait GuiNativeRuntimePump {
    fn pump(&mut self, state: &SorotteGuiShellAppState);

    fn shutdown(&mut self) {}
}

pub(in crate::app) trait GuiQueuedRuntimeOwner {
    /// Gives the UI shutdown path independent access to already launched players.
    fn register_owned_processes(
        &self,
        _scope: &sorotte_player_mpv::managed_process::ManagedMpvShutdownScope,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Compatibility entry point for direct, single-threaded runtime tests.
    /// Production worker code uses `input_changed` and `poll` separately.
    #[cfg(test)]
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SorotteGuiShellAppState) {
        let input = GuiRuntimeInput::from_shell(state);
        self.input_changed(handle, &input);
        self.poll(handle);
    }

    /// Reconciles state submitted by the UI. The threaded bridge calls this
    /// only when the compact runtime input changes.
    fn input_changed(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput);

    /// Advances runtime-owned work and drains queued commands without
    /// rebuilding the UI input projection.
    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle);
}

#[derive(Default)]
pub(in crate::app) struct GuiNoopRuntimePump;

impl GuiNativeRuntimePump for GuiNoopRuntimePump {
    fn pump(&mut self, _state: &SorotteGuiShellAppState) {}
}

#[derive(Default)]
#[cfg(test)]
pub(in crate::app) struct GuiPreviewRuntimeOwner {
    latest_state: Option<SorotteGuiShellAppState>,
}

#[cfg(test)]
impl GuiPreviewRuntimeOwner {
    fn push_preview_response(
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SorotteGuiShellAppState,
        request: GuiRuntimeRequest,
    ) {
        let actions = request.preview_actions_for_state(state);
        if !actions.is_empty() {
            handle.push_actions(actions);
        }
    }
}

#[cfg(test)]
impl GuiQueuedRuntimeOwner for GuiPreviewRuntimeOwner {
    fn input_changed(&mut self, _handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput) {
        self.latest_state = Some(input.to_compatibility_projection());
    }

    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        let Some(state) = self.latest_state.as_ref() else {
            return;
        };
        for request in handle.drain_requests() {
            Self::push_preview_response(handle, state, request);
        }
    }
}
