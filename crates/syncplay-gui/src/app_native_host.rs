use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use eframe::egui;
use syncplay_client_app::app_boundary::commands::{
    LocalInputCommand, LocalOffsetCommand, parse_local_input_command,
};

use super::GuiAppHost;
use super::local_command_dispatch::GuiShellDispatchPlan;
use super::render_egui::{GuiPlaybackPromptKind, GuiWidgetEguiRenderer};
use super::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use super::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiNoopRuntimePump, GuiPendingRoomChangeRequest,
    GuiPreviewRuntimeBridge, GuiQueuedRuntimeOwner,
};
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::{
    GuiQueuedRuntimeBridge, GuiQueuedRuntimeBridgeHandle, GuiThreadedRuntimeOwnerPump,
};
use super::runtime_stack::GuiQueuedSessionTransportHandle;
use super::shell_state::{GuiShellAction, GuiTransientNotificationLevel, SyncplayGuiShellAppState};
use super::startup::syncplay_gui_qsettings_root_from_env;
use super::startup_support::env_trimmed;
use super::support::{nonempty_room_name_text, normalized_editable_text};
use super::ui_state::{GuiPersistedUiState, persist_gui_ui_state_at_root};
#[cfg(test)]
use super::widget_tree::GuiWidgetTextPreviewRenderer;

#[cfg(test)]
#[path = "app_native_host/tests.rs"]
mod tests;

pub(super) struct GuiNativeApp {
    state: SyncplayGuiShellAppState,
    runtime: Box<dyn GuiNativeRuntimeBridge>,
    runtime_pump: Box<dyn GuiNativeRuntimePump>,
    runtime_repaint_handle: Option<GuiQueuedRuntimeBridgeHandle>,
    gui_state_root: Option<PathBuf>,
    test_drop_request: Option<GuiDroppedFilesRequest>,
    playback_prompt: Option<GuiPlaybackPromptKind>,
    playback_prompt_buffer: String,
    playback_prompt_error: Option<String>,
}

impl GuiNativeApp {
    pub(super) fn new(
        creation_context: &eframe::CreationContext<'_>,
        state: SyncplayGuiShellAppState,
        runtime: Box<dyn GuiNativeRuntimeBridge>,
        runtime_pump: Box<dyn GuiNativeRuntimePump>,
        runtime_repaint_handle: Option<GuiQueuedRuntimeBridgeHandle>,
    ) -> Self {
        if let Some(handle) = runtime_repaint_handle.as_ref() {
            let repaint_context = creation_context.egui_ctx.clone();
            handle.set_repaint_notifier(move || {
                repaint_context.request_repaint();
            });
        }
        let test_drop_request = match Self::test_drop_request_from_lookup(&env_trimmed) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("syncplay-gui ignored invalid drag-and-drop test injection: {error}");
                None
            }
        };
        Self {
            state,
            runtime,
            runtime_pump,
            runtime_repaint_handle,
            gui_state_root: syncplay_gui_qsettings_root_from_env(),
            test_drop_request,
            playback_prompt: None,
            playback_prompt_buffer: String::new(),
            playback_prompt_error: None,
        }
    }

    pub(super) fn parse_seek_offset_seconds(value: &str) -> Option<f64> {
        let offset = value.trim().parse::<f64>().ok()?;
        offset.is_finite().then_some(offset)
    }

    pub(super) fn parse_offset_command(value: &str) -> Option<LocalOffsetCommand> {
        let command = parse_local_input_command(&format!("offset {}", value.trim()))?;
        match command {
            LocalInputCommand::SetUserOffset(command) => Some(command),
            _ => None,
        }
    }

    fn preserve_active_playlist_request_index(state: &SyncplayGuiShellAppState) -> Option<usize> {
        (!state.main_window_playlist_selection_is_local)
            .then_some(state.selection.selected_main_window_playlist)
            .flatten()
    }

    pub(super) fn test_drop_request_from_lookup<F>(
        lookup: &F,
    ) -> Result<Option<GuiDroppedFilesRequest>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let Some(raw_paths) = lookup("SYNCPLAY_GUI_TEST_DROP_FILE_PATHS") else {
            return Ok(None);
        };
        let paths = raw_paths
            .split('|')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Ok(None);
        }
        let target = lookup("SYNCPLAY_GUI_TEST_DROP_TARGET")
            .as_deref()
            .map(GuiDroppedFilesTarget::parse)
            .transpose()?
            .unwrap_or(GuiDroppedFilesTarget::Window);
        Ok(Some(GuiDroppedFilesRequest {
            target,
            paths,
            playlist_insert_slot: None,
        }))
    }

    pub(super) fn normalize_dropped_files_request(
        request: GuiDroppedFilesRequest,
    ) -> (Option<GuiDroppedFilesRequest>, Vec<GuiShellAction>) {
        let mut ignored_directories = Vec::new();
        let mut kept_paths = Vec::new();
        for path in request.paths {
            let Some(path) = normalized_editable_text(&path) else {
                continue;
            };
            if !path.contains("://") && Path::new(&path).is_dir() {
                ignored_directories.push(path);
            } else {
                kept_paths.push(path);
            }
        }

        let warnings = if ignored_directories.is_empty() {
            Vec::new()
        } else {
            let message = if ignored_directories.len() == 1 {
                format!(
                    "Dropped folder '{}' was ignored. Desktop drag-and-drop ingest currently supports files only.",
                    ignored_directories[0]
                )
            } else {
                format!(
                    "{} dropped folders were ignored. Desktop drag-and-drop ingest currently supports files only.",
                    ignored_directories.len()
                )
            };
            vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(message),
            ]
        };

        let request = (!kept_paths.is_empty()).then_some(GuiDroppedFilesRequest {
            target: request.target,
            paths: kept_paths,
            playlist_insert_slot: request.playlist_insert_slot,
        });
        (request, warnings)
    }

    pub(super) fn apply_dropped_files_request(&mut self, request: GuiDroppedFilesRequest) -> bool {
        let (request, warning_actions) = Self::normalize_dropped_files_request(request);
        let mut state_changed = false;
        if let Some(request) = request {
            if let Some(path) = request.paths.first() {
                self.state.remember_media_dialog_directory(path);
            }
            for action in self.runtime.actions_for_dropped_files(&self.state, request) {
                state_changed |= self.state.apply(action);
            }
        }
        for action in warning_actions {
            state_changed |= self.state.apply(action);
        }
        state_changed
    }

    pub(super) fn apply_test_drop_request(&mut self, request: GuiDroppedFilesRequest) -> bool {
        let (request, warning_actions) = Self::normalize_dropped_files_request(request);
        let mut state_changed = false;
        if let Some(request) = request {
            if let Some(path) = request.paths.first() {
                self.state.remember_media_dialog_directory(path);
            }
            for action in self.runtime.actions_for_dropped_files(&self.state, request) {
                state_changed |= self.state.apply(action);
            }
        }
        for action in warning_actions {
            state_changed |= self.state.apply(action);
        }
        state_changed
    }

    pub(super) fn open_playback_prompt(&mut self, prompt: GuiPlaybackPromptKind) {
        self.playback_prompt = Some(prompt);
        self.playback_prompt_error = None;
    }

    pub(super) fn close_playback_prompt(&mut self) {
        self.playback_prompt = None;
        self.playback_prompt_buffer.clear();
        self.playback_prompt_error = None;
    }

    pub(super) fn show_playback_prompt(
        &mut self,
        ctx: &egui::Context,
    ) -> (Vec<GuiShellAction>, bool) {
        let Some(prompt) = self.playback_prompt else {
            return (Vec::new(), false);
        };

        let (
            window_title,
            prompt_body,
            button_label,
            hint_text,
            submit_action,
            parse_error_message,
        ) = match prompt {
            GuiPlaybackPromptKind::Seek => (
                "Playback Seek",
                "Enter a seek offset in seconds. Negative values rewind.",
                "Seek",
                "e.g. 12.5 or -5",
                GuiPlaybackPromptKind::Seek,
                "Seek offset must be a finite number of seconds.",
            ),
            GuiPlaybackPromptKind::Offset => (
                "Set Playback Offset",
                "Enter an offset value like 5, +5, -5, or /90.",
                "Set Offset",
                "e.g. +5, -3.5, /90, 12",
                GuiPlaybackPromptKind::Offset,
                "Offset must be a supported Syncplay offset value.",
            ),
        };

        let mut open = true;
        let mut buffer = self.playback_prompt_buffer.clone();
        let mut submit_requested = false;
        let mut cancel_requested = false;

        egui::Window::new(window_title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(prompt_body);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut buffer)
                        .desired_width(200.0)
                        .hint_text(hint_text),
                );
                let submitted =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if let Some(error) = self.playback_prompt_error.as_deref() {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(button_label).clicked() {
                        submit_requested = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_requested = true;
                    }
                });
                if submitted {
                    submit_requested = true;
                }
            });

        let mut local_state_changed = false;
        if buffer != self.playback_prompt_buffer {
            self.playback_prompt_buffer = buffer;
            if self.playback_prompt_error.take().is_some() {
                local_state_changed = true;
            }
        }

        if submit_requested {
            let actions = match submit_action {
                GuiPlaybackPromptKind::Seek => {
                    Self::parse_seek_offset_seconds(&self.playback_prompt_buffer)
                        .map(|offset_seconds| self.runtime.actions_for_seek_offset(offset_seconds))
                }
                GuiPlaybackPromptKind::Offset => {
                    Self::parse_offset_command(&self.playback_prompt_buffer)
                        .map(|command| self.runtime.actions_for_set_offset(command))
                }
            };
            if let Some(actions) = actions {
                self.close_playback_prompt();
                return (actions, true);
            }
            self.playback_prompt_error = Some(parse_error_message.to_owned());
            return (Vec::new(), true);
        }

        if !open || cancel_requested {
            self.close_playback_prompt();
            return (Vec::new(), true);
        }

        (Vec::new(), local_state_changed)
    }
}

impl eframe::App for GuiNativeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut renderer = GuiWidgetEguiRenderer::default();
        self.state.render_shell_widgets(&mut renderer);
        let show_manual_pending_controls = self.runtime.shows_manual_pending_controls();
        let dispatch_plan = GuiShellDispatchPlan::from_shell_actions(
            &self.state,
            renderer.show(ctx, &self.state, show_manual_pending_controls),
        );
        let close_requested = renderer.take_close_requested();
        let selected_media_files = renderer.take_selected_media_files();
        let dropped_files_request = renderer.take_dropped_files_request();
        let pending_completion_requested = renderer.take_pending_completion_requested();
        let pending_cancel_requested = renderer.take_pending_cancel_requested();
        let playlist_entries_before_actions = self.state.current_shared_playlist_entries();
        let mut room_change_requests = Vec::new();
        let mut main_window_user_media_requests = Vec::new();
        let mut main_window_user_folder_requests = Vec::new();
        let mut main_window_user_ready_requests = Vec::new();
        let mut controller_auth_requests = Vec::new();
        let mut requested_local_ready = None;
        let mut playlist_entry_draft = self.state.new_playlist_entry_draft.clone();
        let mut selected_playlist_index = self.state.selection.selected_main_window_playlist;
        let mut playlist_entry_commits = Vec::new();
        let mut appended_playlist_entries = Vec::new();
        let mut playlist_activation_requests = Vec::new();
        let mut playlist_deletions = Vec::new();
        let mut playlist_reorder_requested = false;
        let mut playlist_replace_requested = false;
        let mut playlist_undo_requested = false;
        let mut playlist_shuffle_remaining_requested = false;
        let mut playlist_shuffle_entire_requested = false;
        let mut requested_playback_prompt = None;
        let mut requested_undo_seek = false;
        let mut requested_autoplay_state = None;
        let mut requested_autoplay_threshold = None;
        for action in &dispatch_plan.shell_actions {
            match action {
                GuiShellAction::JoinMainWindowRoom(room) => {
                    if let Some(room) = nonempty_room_name_text(room) {
                        room_change_requests.push(GuiPendingRoomChangeRequest::Join {
                            requested_room: room.to_owned(),
                        });
                    }
                }
                GuiShellAction::LeaveMainWindowRoom => {
                    room_change_requests.push(GuiPendingRoomChangeRequest::ReturnToDefault {
                        previous_room: self.state.main_window.room_name.clone(),
                    })
                }
                GuiShellAction::RequestMainWindowUserMediaOpen(target) => {
                    if let Some(target) = normalized_editable_text(target) {
                        main_window_user_media_requests.push(target.to_owned());
                    }
                }
                GuiShellAction::RequestMainWindowUserContainingFolderOpen(target) => {
                    if let Some(target) = normalized_editable_text(target) {
                        main_window_user_folder_requests.push(target.to_owned());
                    }
                }
                GuiShellAction::RequestMainWindowUserReady { username, ready } => {
                    if let Some(username) = normalized_editable_text(username) {
                        main_window_user_ready_requests.push((username.to_owned(), *ready));
                    }
                }
                GuiShellAction::RequestControllerAuth { room, password } => {
                    if let (Some(room), Some(password)) = (
                        nonempty_room_name_text(room),
                        normalized_editable_text(password),
                    ) {
                        controller_auth_requests.push((room.to_owned(), password.to_owned()));
                    }
                }
                GuiShellAction::AnnounceLocalUserReady => requested_local_ready = Some(true),
                GuiShellAction::AnnounceLocalUserNotReady => requested_local_ready = Some(false),
                GuiShellAction::UpdateNewPlaylistEntryDraft(buffer) => {
                    playlist_entry_draft = buffer.clone();
                }
                GuiShellAction::CommitNewPlaylistEntry => {
                    if let Some(entry) = normalized_editable_text(&playlist_entry_draft) {
                        playlist_entry_commits.push(entry.to_owned());
                    }
                }
                GuiShellAction::AppendSharedPlaylistEntries(entries) => {
                    appended_playlist_entries.push(entries.clone());
                }
                GuiShellAction::ReplaceSharedPlaylistEntries(_)
                | GuiShellAction::LoadSharedPlaylistFromFile { .. } => {
                    playlist_replace_requested = true;
                }
                GuiShellAction::SelectMainWindowPlaylist(index) => {
                    selected_playlist_index = Some(*index);
                }
                GuiShellAction::ActivateMainWindowPlaylist(index) => {
                    selected_playlist_index = Some(*index);
                    playlist_activation_requests.push(*index);
                }
                GuiShellAction::MoveMainWindowPlaylistRow { .. } => {
                    playlist_reorder_requested = true;
                }
                GuiShellAction::RemoveSelectedMainWindowPlaylist => {
                    if let Some(index) = selected_playlist_index {
                        playlist_deletions.push(index);
                    }
                }
                GuiShellAction::MoveSelectedMainWindowPlaylistUp
                | GuiShellAction::MoveSelectedMainWindowPlaylistDown => {
                    playlist_reorder_requested = true;
                }
                GuiShellAction::UndoSharedPlaylistChange => {
                    playlist_undo_requested = true;
                }
                GuiShellAction::ShuffleRemainingSharedPlaylist => {
                    playlist_shuffle_remaining_requested = true;
                }
                GuiShellAction::ShuffleEntireSharedPlaylist => {
                    playlist_shuffle_entire_requested = true;
                }
                GuiShellAction::RequestSeekPrompt => {
                    requested_playback_prompt = Some(GuiPlaybackPromptKind::Seek);
                }
                GuiShellAction::RequestOffsetPrompt => {
                    requested_playback_prompt = Some(GuiPlaybackPromptKind::Offset);
                }
                GuiShellAction::RequestPlaybackUndoSeek => {
                    requested_undo_seek = true;
                }
                GuiShellAction::AnnounceAutoplayState(active) => {
                    requested_autoplay_state = Some(*active);
                }
                GuiShellAction::AnnounceAutoplayThreshold(threshold) => {
                    requested_autoplay_threshold = Some(*threshold);
                }
                _ => {}
            }
        }
        if let Some(prompt) = requested_playback_prompt {
            self.open_playback_prompt(prompt);
        }
        let mut state_changed = false;
        for action in dispatch_plan.shell_actions {
            state_changed |= self.state.apply(action);
        }
        for request in dispatch_plan.runtime_requests {
            for action in self.runtime.dispatch_runtime_request(&self.state, request) {
                state_changed |= self.state.apply(action);
            }
        }
        for request in room_change_requests {
            let runtime_actions = match request {
                GuiPendingRoomChangeRequest::Join { requested_room } => self
                    .runtime
                    .actions_for_room_join(&self.state, requested_room),
                GuiPendingRoomChangeRequest::ReturnToDefault { .. } => {
                    self.runtime.actions_for_room_leave(&self.state)
                }
            };
            for action in runtime_actions {
                state_changed |= self.state.apply(action);
            }
        }
        for target in main_window_user_media_requests {
            for action in self
                .runtime
                .actions_for_main_window_user_media_open(&self.state, target)
            {
                state_changed |= self.state.apply(action);
            }
        }
        for target in main_window_user_folder_requests {
            for action in self
                .runtime
                .actions_for_main_window_user_folder_open(&self.state, target)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if let Some(ready) = requested_local_ready {
            for action in self
                .runtime
                .actions_for_local_readiness_change(&self.state, ready)
            {
                state_changed |= self.state.apply(action);
            }
        }
        for (username, ready) in main_window_user_ready_requests {
            for action in self.runtime.actions_for_main_window_user_readiness_change(
                &self.state,
                username,
                ready,
            ) {
                state_changed |= self.state.apply(action);
            }
        }
        for (room, password) in controller_auth_requests {
            for action in
                self.runtime
                    .actions_for_controller_auth_request(&self.state, room, password)
            {
                state_changed |= self.state.apply(action);
            }
        }
        let mut dispatched_playlist_entries = playlist_entries_before_actions
            .into_iter()
            .collect::<BTreeSet<_>>();
        for entry in playlist_entry_commits {
            if !dispatched_playlist_entries.insert(entry.clone()) {
                continue;
            }
            for action in self
                .runtime
                .actions_for_playlist_entry_commit(&self.state, entry, false)
            {
                state_changed |= self.state.apply(action);
            }
        }
        for entries in appended_playlist_entries {
            for entry in entries {
                if !dispatched_playlist_entries.insert(entry.clone()) {
                    continue;
                }
                for action in
                    self.runtime
                        .actions_for_playlist_entry_commit(&self.state, entry, false)
                {
                    state_changed |= self.state.apply(action);
                }
            }
        }
        for index in playlist_activation_requests {
            for action in self
                .runtime
                .actions_for_playlist_activation(&self.state, index)
            {
                state_changed |= self.state.apply(action);
            }
        }
        for index in playlist_deletions {
            for action in self
                .runtime
                .actions_for_playlist_entry_removal(&self.state, index)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if playlist_replace_requested {
            let playlist = self
                .state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.clone())
                .collect();
            for action in self.runtime.actions_for_playlist_reorder(
                &self.state,
                playlist,
                Self::preserve_active_playlist_request_index(&self.state),
            ) {
                state_changed |= self.state.apply(action);
            }
        }
        if playlist_undo_requested {
            for action in self.runtime.actions_for_playlist_undo(&self.state) {
                state_changed |= self.state.apply(action);
            }
        }
        if playlist_shuffle_remaining_requested {
            for action in self
                .runtime
                .actions_for_playlist_shuffle_remaining(&self.state)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if playlist_shuffle_entire_requested {
            for action in self
                .runtime
                .actions_for_playlist_shuffle_entire(&self.state)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if playlist_reorder_requested {
            let playlist = self
                .state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.clone())
                .collect();
            for action in self.runtime.actions_for_playlist_reorder(
                &self.state,
                playlist,
                Self::preserve_active_playlist_request_index(&self.state),
            ) {
                state_changed |= self.state.apply(action);
            }
        }
        if requested_undo_seek {
            for action in self.runtime.actions_for_undo_seek() {
                state_changed |= self.state.apply(action);
            }
        }
        if let Some(autoplay_state) = requested_autoplay_state {
            for action in self
                .runtime
                .actions_for_autoplay_enabled_change(autoplay_state)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if let Some(autoplay_threshold) = requested_autoplay_threshold {
            for action in self
                .runtime
                .actions_for_autoplay_threshold_change(autoplay_threshold)
            {
                state_changed |= self.state.apply(action);
            }
        }
        for action in self.runtime.drain_runtime_actions() {
            state_changed |= self.state.apply(action);
        }
        if let Some(paths) = selected_media_files {
            if let Some(path) = paths.first() {
                self.state.remember_media_dialog_directory(path);
            }
            for action in self
                .runtime
                .actions_for_selected_media_files(&self.state, paths)
            {
                state_changed |= self.state.apply(action);
            }
        }
        if let Some(request) = self.test_drop_request.take() {
            state_changed |= self.apply_test_drop_request(request);
        }
        if let Some(request) = dropped_files_request {
            state_changed |= self.apply_dropped_files_request(request);
        }
        let auto_pending_completion_requested =
            !show_manual_pending_controls && self.state.pending_operation.is_some();
        if pending_completion_requested || auto_pending_completion_requested {
            for action in self.runtime.actions_for_pending_completion(&self.state) {
                state_changed |= self.state.apply(action);
            }
        }
        if pending_cancel_requested {
            for action in self.runtime.actions_for_pending_cancel(&self.state) {
                state_changed |= self.state.apply(action);
            }
        }
        let (playback_prompt_actions, playback_prompt_state_changed) =
            self.show_playback_prompt(ctx);
        for action in playback_prompt_actions {
            state_changed |= self.state.apply(action);
        }
        self.runtime_pump.pump(&self.state);
        for action in self.runtime.drain_runtime_actions() {
            state_changed |= self.state.apply(action);
        }
        if close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if state_changed
            || requested_playback_prompt.is_some()
            || playback_prompt_state_changed
            || pending_completion_requested
            || pending_cancel_requested
        {
            ctx.request_repaint();
        }
    }
}

impl Drop for GuiNativeApp {
    fn drop(&mut self) {
        if let Some(handle) = self.runtime_repaint_handle.as_ref() {
            handle.clear_repaint_notifier();
        }
        let Some(root) = self.gui_state_root.as_deref() else {
            return;
        };
        let persisted_state = GuiPersistedUiState::from_shell_state(&self.state);
        if let Err(error) = persist_gui_ui_state_at_root(root, &persisted_state) {
            eprintln!("syncplay-gui failed to persist legacy GUI state: {error}");
        }
    }
}

pub(super) struct GuiEframeNativeHost {
    runtime: Option<Box<dyn GuiNativeRuntimeBridge>>,
    runtime_pump: Option<Box<dyn GuiNativeRuntimePump>>,
    runtime_repaint_handle: Option<GuiQueuedRuntimeBridgeHandle>,
}

impl GuiEframeNativeHost {
    pub(super) fn native_options() -> eframe::NativeOptions {
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Syncplay GUI")
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([960.0, 640.0])
                .with_drag_and_drop(true),
            ..Default::default()
        }
    }

    pub(super) fn with_runtime_and_pump(
        runtime: Box<dyn GuiNativeRuntimeBridge>,
        runtime_pump: Box<dyn GuiNativeRuntimePump>,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            runtime_pump: Some(runtime_pump),
            runtime_repaint_handle: None,
        }
    }

    pub(super) fn with_runtime(runtime: Box<dyn GuiNativeRuntimeBridge>) -> Self {
        Self::with_runtime_and_pump(runtime, Box::<GuiNoopRuntimePump>::default())
    }

    pub(super) fn with_queued_runtime_owner<TOwner>(
        show_manual_pending_controls: bool,
        owner: TOwner,
    ) -> Self
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        let (runtime, handle) =
            GuiQueuedRuntimeBridge::new_with_manual_pending_controls(show_manual_pending_controls);
        let repaint_handle = handle.clone();
        let mut host = Self::with_runtime_and_pump(
            Box::new(runtime),
            Box::new(GuiThreadedRuntimeOwnerPump::new(handle, owner)),
        );
        host.runtime_repaint_handle = Some(repaint_handle);
        host
    }

    pub(super) fn with_queued_preview_runtime_for_config_path(
        config_path: Option<PathBuf>,
    ) -> Self {
        Self::with_queued_runtime_owner(
            false,
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path),
        )
    }

    pub(super) fn with_queued_preview_runtime() -> Self {
        Self::with_queued_preview_runtime_for_config_path(None)
    }

    pub(super) fn with_client_core_chat_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        config_path: Option<PathBuf>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        let (owner, session_transport) =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_session_runtime(username, room)?;
        Ok((
            Self::with_queued_runtime_owner(false, owner),
            session_transport,
        ))
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_session(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        Self::with_client_core_chat_session_for_config_path(username, room, None)
    }

    pub(super) fn with_client_core_chat_loopback_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        config_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let owner =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_loopback_session_runtime(username, room)?;
        Ok(Self::with_queued_runtime_owner(false, owner))
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_loopback_session(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        Self::with_client_core_chat_loopback_session_for_config_path(username, room, None)
    }

    pub(super) fn with_client_core_chat_tcp_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
        config_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let owner =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_tcp_session_runtime(username, room, host_arg)?;
        Ok(Self::with_queued_runtime_owner(false, owner))
    }

    #[allow(dead_code)]
    pub(super) fn with_client_core_chat_tcp_session(
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
    ) -> Result<Self, String> {
        Self::with_client_core_chat_tcp_session_for_config_path(username, room, host_arg, None)
    }

    #[allow(dead_code)]
    pub(super) fn with_queued_runtime() -> (Self, GuiQueuedRuntimeBridgeHandle) {
        let (runtime, handle) = GuiQueuedRuntimeBridge::new();
        let mut host = Self::with_runtime(Box::new(runtime));
        host.runtime_repaint_handle = Some(handle.clone());
        (host, handle)
    }
}

impl Default for GuiEframeNativeHost {
    fn default() -> Self {
        Self::with_queued_preview_runtime()
    }
}

impl GuiAppHost for GuiEframeNativeHost {
    type Output = eframe::Result<()>;

    fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
        let runtime = self
            .runtime
            .take()
            .unwrap_or_else(|| Box::<GuiPreviewRuntimeBridge>::default());
        let runtime_pump = self
            .runtime_pump
            .take()
            .unwrap_or_else(|| Box::<GuiNoopRuntimePump>::default());
        let runtime_repaint_handle = self.runtime_repaint_handle.take();
        eframe::run_native(
            "Syncplay GUI",
            Self::native_options(),
            Box::new(move |creation_context| {
                Ok(Box::new(GuiNativeApp::new(
                    creation_context,
                    state,
                    runtime,
                    runtime_pump,
                    runtime_repaint_handle,
                )))
            }),
        )
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct GuiTextPreviewHost;

#[cfg(test)]
impl GuiAppHost for GuiTextPreviewHost {
    type Output = String;

    fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
        let mut renderer = GuiWidgetTextPreviewRenderer::default();
        state.render_shell_widgets(&mut renderer);
        format!(
            "{}\n\n[Widget Tree]\n{}",
            state.render_lines().join("\n"),
            renderer.finish()
        )
    }
}
