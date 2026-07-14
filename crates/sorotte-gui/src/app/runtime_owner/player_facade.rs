use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn open_media_unavailable_message(&self, selected_paths: &[String]) -> String {
        self.open_media_unavailable_message_impl(selected_paths)
    }

    pub(in crate::app) fn shared_playlist_open_dispatch_for_paths(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        Self::shared_playlist_open_dispatch_for_paths_impl(paths)
    }

    pub(super) fn import_shared_playlist_file_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        path: String,
        shuffled: bool,
    ) {
        self.import_shared_playlist_file_runtime_impl(handle, projected_state, path, shuffled);
    }

    pub(super) fn seek_unavailable_message(&self, offset_seconds: f64) -> String {
        self.seek_unavailable_message_impl(offset_seconds)
    }

    pub(super) fn toggle_pause_unavailable_message(&self) -> String {
        self.toggle_pause_unavailable_message_impl()
    }

    pub(super) fn send_chat_unavailable_message(&self) -> String {
        self.send_chat_unavailable_message_impl()
    }

    pub(super) fn push_player_success(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        Self::push_player_success_impl(handle, message)
    }

    pub(super) fn push_player_error(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        Self::push_player_error_impl(handle, message)
    }

    pub(super) fn open_media_files_through_attached_player(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        self.open_media_files_through_attached_player_impl(handle, paths)
    }

    pub(super) fn open_main_window_user_media_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        self.open_main_window_user_media_runtime_impl(handle, projected_state, target)
    }

    pub(super) fn open_main_window_user_containing_folder_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        self.open_main_window_user_containing_folder_runtime_impl(handle, projected_state, target)
    }

    pub(super) fn open_stream_helper_install_location_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        path: PathBuf,
    ) {
        self.open_stream_helper_install_location_runtime_impl(handle, projected_state, path)
    }

    pub(super) fn open_media_files_through_shared_playlist_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        paths: Vec<String>,
        playlist_insert_slot: Option<usize>,
    ) {
        self.open_media_files_through_shared_playlist_runtime_impl(
            handle,
            projected_state,
            paths,
            playlist_insert_slot,
        )
    }

    pub(super) fn emit_gui_actions_to_attached_player(&mut self, actions: &[GuiShellAction]) {
        self.emit_gui_actions_to_attached_player_impl(actions)
    }

    pub(super) fn drain_player_chat_input(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        self.drain_player_chat_input_impl(handle, projected_state)
    }

    pub(super) fn refresh_player_state(&mut self) {
        self.refresh_player_state_impl()
    }

    pub(super) fn player_target_position_seconds_for_global_position(
        &self,
        global_position_seconds: f64,
    ) -> f64 {
        self.player_target_position_seconds_for_global_position_impl(global_position_seconds)
    }

    pub(super) fn sync_manual_seek_into_detached_session(
        &mut self,
        state: &SorotteGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<bool, String> {
        self.sync_manual_seek_into_detached_session_impl(
            state,
            previous_position_seconds,
            target_position_seconds,
        )
    }

    pub(super) fn apply_playback_pause_change_with_detached_session(
        &mut self,
        state: &SorotteGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(bool, Option<String>), String> {
        self.apply_playback_pause_change_with_detached_session_impl(
            state,
            previous_paused,
            target_paused,
        )
    }

    pub(super) fn undo_seek_target_position_from_detached_session(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.undo_seek_target_position_from_detached_session_impl(state)
    }

    pub(super) fn commit_undo_seek_into_detached_session(
        &mut self,
        state: &SorotteGuiShellAppState,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.commit_undo_seek_into_detached_session_impl(state, target_position_seconds)
    }

    pub(in crate::app) fn player_local_file_playlist_entries(&self) -> Vec<String> {
        self.player_local_file_playlist_entries_impl()
    }

    pub(in crate::app) fn command_availability_for_runtime_state(
        &self,
        state: &SorotteGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        self.command_availability_for_runtime_state_impl(state, player_attached)
    }
}
