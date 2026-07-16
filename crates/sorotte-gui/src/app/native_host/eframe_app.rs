use super::*;

impl eframe::App for GuiNativeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Self::apply_test_theme_override_from_lookup(ctx, &env_trimmed);
        let mut renderer = GuiWidgetEguiRenderer::default();
        self.state.render_shell_widgets(&mut renderer);
        let show_manual_pending_controls = self.runtime.shows_manual_pending_controls();
        let dispatch_plan = GuiShellDispatchPlan::from_shell_actions(
            &self.state,
            renderer.show(ctx, &self.state, show_manual_pending_controls),
        );
        let mut close_requested = renderer.take_close_requested();
        let mut selected_media_files = renderer.take_selected_media_files();
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
        let mut selected_playlist_index = self.state.selection.selected_main_window_playlist;
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
                        normalized_editable_text(password.expose_secret()),
                    ) {
                        controller_auth_requests.push((room.to_owned(), password.to_owned()));
                    }
                }
                GuiShellAction::AnnounceLocalUserReady => requested_local_ready = Some(true),
                GuiShellAction::AnnounceLocalUserNotReady => requested_local_ready = Some(false),
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
                GuiShellAction::AnnounceAutoplayState(active) => {
                    requested_autoplay_state = Some(*active);
                }
                GuiShellAction::AnnounceAutoplayThreshold(threshold) => {
                    requested_autoplay_threshold = Some(*threshold);
                }
                _ => {}
            }
        }
        let mut state_changed = false;
        for request in dispatch_plan.pre_shell_runtime_requests {
            for action in self.runtime.dispatch_runtime_request(&self.state, request) {
                state_changed |= self.state.apply(action);
            }
        }
        for action in dispatch_plan.shell_actions {
            let native_effect = Self::native_effect_for_applied_action(&action, true);
            let action_applied = self.state.apply(action);
            state_changed |= action_applied;
            if !action_applied {
                continue;
            }
            match native_effect {
                Some(GuiNativeShellEffect::PickMediaFiles) => {
                    selected_media_files = GuiWidgetEguiRenderer::pick_media_files(&self.state);
                }
                Some(GuiNativeShellEffect::CloseWindow) => close_requested = true,
                Some(GuiNativeShellEffect::OpenPlaybackPrompt(prompt)) => {
                    requested_playback_prompt = Some(prompt);
                }
                Some(GuiNativeShellEffect::RequestUndoSeek) => requested_undo_seek = true,
                Some(GuiNativeShellEffect::OpenHelp) => {
                    ctx.open_url(egui::OpenUrl::new_tab(MenuActionId::help_url()));
                }
                None => {}
            }
        }
        if let Some(prompt) = requested_playback_prompt {
            self.open_playback_prompt(prompt);
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
            if Self::action_requests_app_close(&action) {
                close_requested = true;
            }
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
            if Self::action_requests_app_close(&action) {
                close_requested = true;
            }
            state_changed |= self.state.apply(action);
        }
        if close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if state_changed
            || requested_playback_prompt.is_some()
            || playback_prompt_state_changed
            || pending_completion_requested
            || auto_pending_completion_requested
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
            eprintln!("sorotte-gui failed to persist legacy GUI state: {error}");
        }
    }
}
