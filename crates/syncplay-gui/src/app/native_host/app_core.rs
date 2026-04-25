use super::*;

impl GuiNativeApp {
    pub(in crate::app) fn new(
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

    pub(in crate::app) fn parse_seek_offset_seconds(value: &str) -> Option<f64> {
        let offset = value.trim().parse::<f64>().ok()?;
        offset.is_finite().then_some(offset)
    }

    pub(in crate::app) fn parse_offset_command(value: &str) -> Option<LocalOffsetCommand> {
        let command = parse_local_input_command(&format!("offset {}", value.trim()))?;
        match command {
            LocalInputCommand::SetUserOffset(command) => Some(command),
            _ => None,
        }
    }

    pub(in crate::app::native_host) fn preserve_active_playlist_request_index(
        state: &SyncplayGuiShellAppState,
    ) -> Option<usize> {
        (!state.main_window_playlist_selection_is_local)
            .then_some(state.selection.selected_main_window_playlist)
            .flatten()
    }

    pub(in crate::app) fn test_drop_request_from_lookup<F>(
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

    pub(in crate::app) fn normalize_dropped_files_request(
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

    pub(in crate::app) fn apply_dropped_files_request(
        &mut self,
        request: GuiDroppedFilesRequest,
    ) -> bool {
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

    pub(in crate::app) fn apply_test_drop_request(
        &mut self,
        request: GuiDroppedFilesRequest,
    ) -> bool {
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

    pub(in crate::app) fn open_playback_prompt(&mut self, prompt: GuiPlaybackPromptKind) {
        self.playback_prompt = Some(prompt);
        self.playback_prompt_error = None;
    }

    pub(in crate::app) fn close_playback_prompt(&mut self) {
        self.playback_prompt = None;
        self.playback_prompt_buffer.clear();
        self.playback_prompt_error = None;
    }

    pub(in crate::app) fn show_playback_prompt(
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
