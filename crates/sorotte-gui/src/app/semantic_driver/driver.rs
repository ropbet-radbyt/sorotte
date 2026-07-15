use super::super::{
    GuiDroppedFilesTarget, GuiInteractionRuntimeSnapshot, GuiPendingCompletionRequest,
    GuiPendingOperationKind, GuiPersistedConfigRuntimeOwner, GuiPreviewRuntimeBridge,
    GuiQueuedRuntimeBridgeHandle, GuiRuntimeRequest, GuiShellAction, GuiWidgetEguiRenderer,
    GuiWidgetKind, GuiWidgetNode, MainWindowRuntimeSnapshot, SorotteGuiShellAppState,
    StoredClientSettingsMvp,
};
use super::super::{GuiNativeRuntimeBridge, local_command_dispatch::GuiShellDispatchPlan};
use super::GuiSemanticStep;

pub(in crate::app) struct GuiSemanticDriver {
    state: SorotteGuiShellAppState,
    media_fixture_root: Option<std::path::PathBuf>,
}

impl GuiSemanticDriver {
    fn new(state: SorotteGuiShellAppState) -> Self {
        Self {
            state,
            media_fixture_root: None,
        }
    }

    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        Self::new(SorotteGuiShellAppState::from_stored_settings(settings))
    }

    #[cfg(test)]
    pub(in crate::app) fn state(&self) -> &SorotteGuiShellAppState {
        &self.state
    }

    pub(in crate::app) fn active_view_label(&self) -> &'static str {
        self.state.active_view.label()
    }

    pub(in crate::app) fn active_modal_label(&self) -> &'static str {
        self.state
            .open_modal
            .map(|modal| modal.label())
            .unwrap_or("none")
    }

    pub(in crate::app) fn pending_operation_label(&self) -> &'static str {
        self.state
            .pending_operation
            .as_ref()
            .map(|operation| operation.kind.label())
            .unwrap_or("none")
    }

    pub(in crate::app) fn shell_tree(&self) -> GuiWidgetNode {
        self.state.shell_widget_tree()
    }

    pub(in crate::app) fn widget_count(&self) -> usize {
        self.shell_tree().node_count()
    }

    pub(in crate::app) fn widget(&self, widget_id: &str) -> Result<GuiWidgetNode, String> {
        self.shell_tree()
            .find(widget_id)
            .cloned()
            .ok_or_else(|| format!("expected widget {widget_id} to exist"))
    }

    fn apply_actions(&mut self, actions: impl IntoIterator<Item = GuiShellAction>) {
        for action in actions {
            self.state.apply(action);
        }
    }

    fn dispatch_shell_actions(&mut self, actions: Vec<GuiShellAction>) {
        let dispatch_plan = GuiShellDispatchPlan::from_shell_actions(&self.state, actions);
        let mut runtime = GuiPreviewRuntimeBridge;
        for request in dispatch_plan.pre_shell_runtime_requests {
            self.apply_actions(GuiNativeRuntimeBridge::dispatch_runtime_request(
                &mut runtime,
                &self.state,
                request,
            ));
        }
        self.apply_actions(dispatch_plan.shell_actions);
        for request in dispatch_plan.runtime_requests {
            self.apply_actions(GuiNativeRuntimeBridge::dispatch_runtime_request(
                &mut runtime,
                &self.state,
                request,
            ));
        }
    }

    fn activate_widget(&mut self, widget_id: &str) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        let actions = match widget.kind {
            GuiWidgetKind::Layout | GuiWidgetKind::Panel => {
                GuiWidgetEguiRenderer::action_for_surface_node(&widget)
                    .into_iter()
                    .collect::<Vec<_>>()
            }
            GuiWidgetKind::Checkbox => {
                let next_value = widget.value.as_deref() != Some("yes");
                GuiWidgetEguiRenderer::action_for_checkbox_node(&self.state, &widget, next_value)
                    .into_iter()
                    .collect::<Vec<_>>()
            }
            GuiWidgetKind::Button => {
                if widget.id == "media-search:command:browse" {
                    return Err(
                        "semantic driver does not depend on native folder dialogs".to_owned()
                    );
                }
                GuiWidgetEguiRenderer::actions_for_button_node(&self.state, &widget)
            }
            GuiWidgetKind::ListItem => GuiWidgetEguiRenderer::action_for_list_item_node(&widget)
                .into_iter()
                .collect::<Vec<_>>(),
            _ => {
                return Err(format!(
                    "widget {widget_id} with kind {:?} does not support semantic activation",
                    widget.kind
                ));
            }
        };
        if actions.is_empty() {
            return Err(format!(
                "semantic activation should map {widget_id} to at least one action",
            ));
        }
        self.dispatch_shell_actions(actions);
        Ok(())
    }

    fn enter_text(&mut self, widget_id: &str, value: &str, submit: bool) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        let Some(actions) = GuiWidgetEguiRenderer::actions_for_text_input_node(
            &self.state,
            &widget,
            value,
            true,
            submit,
        ) else {
            return Err(format!(
                "semantic text entry should map {widget_id} to at least one action",
            ));
        };
        self.dispatch_shell_actions(actions);
        Ok(())
    }

    fn assert_widget_value(&self, widget_id: &str, value: Option<&str>) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        if widget.value.as_deref() != value {
            return Err(format!(
                "widget {widget_id} value mismatch: expected {:?}, got {:?}",
                value,
                widget.value.as_deref()
            ));
        }
        Ok(())
    }

    fn assert_widget_label(&self, widget_id: &str, label: &str) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        if widget.label != label {
            return Err(format!(
                "widget {widget_id} label mismatch: expected {:?}, got {:?}",
                label, widget.label
            ));
        }
        Ok(())
    }

    fn assert_widget_selected(&self, widget_id: &str, selected: bool) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        if widget.selected != selected {
            return Err(format!(
                "widget {widget_id} selected mismatch: expected {selected}, got {}",
                widget.selected
            ));
        }
        Ok(())
    }

    fn assert_widget_enabled(&self, widget_id: &str, enabled: bool) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        if widget.enabled != enabled {
            return Err(format!(
                "widget {widget_id} enabled mismatch: expected {enabled}, got {}",
                widget.enabled
            ));
        }
        Ok(())
    }

    fn assert_pending(&self, pending: Option<GuiPendingOperationKind>) -> Result<(), String> {
        let actual = self
            .state
            .pending_operation
            .as_ref()
            .map(|operation| operation.kind);
        if actual != pending {
            return Err(format!(
                "pending-operation mismatch: expected {pending:?}, got {actual:?}",
            ));
        }
        Ok(())
    }

    fn complete_pending(&mut self) -> Result<(), String> {
        let actions = GuiPreviewRuntimeBridge::preview_pending_completion_actions(&self.state);
        if actions.is_empty() {
            return Err(
                "semantic completion should produce at least one pending action".to_owned(),
            );
        }
        self.apply_actions(actions);
        Ok(())
    }

    fn open_media_files(&mut self, paths: Vec<String>) -> Result<(), String> {
        let paths = self.materialize_media_fixture_paths(paths)?;
        let actions = GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            Some(&self.state),
            paths,
            self.state.shared_playlist_events_enabled(),
            None,
        );
        if actions.is_empty() {
            return Err(
                "semantic media-open should produce at least one runtime preview action".to_owned(),
            );
        }
        self.apply_actions(actions);
        Ok(())
    }

    fn drop_media_files(
        &mut self,
        target: GuiDroppedFilesTarget,
        paths: Vec<String>,
    ) -> Result<(), String> {
        let paths = self.materialize_media_fixture_paths(paths)?;
        let playlist_insert_slot = matches!(target, GuiDroppedFilesTarget::Playlist)
            .then_some(self.state.main_window.playlist.len());
        let actions = GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            Some(&self.state),
            paths,
            target.load_into_shared_playlist(&self.state),
            playlist_insert_slot,
        );
        if actions.is_empty() {
            return Err(
                "semantic dropped-media ingest should produce at least one runtime preview action"
                    .to_owned(),
            );
        }
        self.apply_actions(actions);
        Ok(())
    }

    fn materialize_media_fixture_paths(
        &mut self,
        paths: Vec<String>,
    ) -> Result<Vec<String>, String> {
        paths
            .into_iter()
            .map(|path| {
                let Some(relative_path) = path.strip_prefix("fixture-media://") else {
                    return Ok(path);
                };
                let relative_path = std::path::Path::new(relative_path);
                if relative_path.as_os_str().is_empty()
                    || relative_path.is_absolute()
                    || relative_path
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Err(format!(
                        "semantic media fixture path must be a safe relative path: {path}"
                    ));
                }
                let root = self.media_fixture_root.get_or_insert_with(|| {
                    let unique_suffix = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_nanos())
                        .unwrap_or_default();
                    std::env::temp_dir().join(format!(
                        "sorotte-gui-semantic-media-{}-{unique_suffix}",
                        std::process::id()
                    ))
                });
                let fixture_path = root.join(relative_path);
                if let Some(parent) = fixture_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        format!("semantic media fixture directory could not be created: {error}")
                    })?;
                }
                std::fs::write(&fixture_path, b"semantic media fixture").map_err(|error| {
                    format!("semantic media fixture could not be written: {error}")
                })?;
                Ok(fixture_path.to_string_lossy().into_owned())
            })
            .collect()
    }

    fn complete_pending_via_runtime(&mut self) -> Result<(), String> {
        let Some(request) = GuiPendingCompletionRequest::from_state(&self.state) else {
            return Err(
                "semantic runtime completion requires an active pending operation".to_owned(),
            );
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(request));
        owner.pump_compatibility_state(&handle, &self.state);
        let actions = handle.drain_actions();
        if actions.is_empty() {
            return Err(
                "semantic runtime completion should produce at least one runtime action".to_owned(),
            );
        }
        self.apply_actions(actions);
        Ok(())
    }

    fn cancel_pending(&mut self) -> Result<(), String> {
        let actions = GuiPreviewRuntimeBridge::preview_pending_cancel_actions(&self.state);
        if actions.is_empty() {
            return Err("semantic cancel should produce at least one pending action".to_owned());
        }
        self.apply_actions(actions);
        Ok(())
    }

    pub(in crate::app::semantic_driver) fn run_steps(
        &mut self,
        steps: &[GuiSemanticStep],
    ) -> Result<(), String> {
        for step in steps {
            match step {
                GuiSemanticStep::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                    self.apply_actions([GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                        snapshot.clone(),
                    )]);
                }
                GuiSemanticStep::ApplyMainWindowReadinessPresentation(readiness) => {
                    let mut snapshot =
                        MainWindowRuntimeSnapshot::from_shell_state(&self.state.main_window);
                    let Some(user) = snapshot
                        .users
                        .iter_mut()
                        .find(|user| user.username == readiness.username)
                    else {
                        return Err(format!(
                            "semantic readiness update references missing user {:?}",
                            readiness.username
                        ));
                    };
                    user.is_ready = readiness.room_ready;
                    snapshot
                        .readiness
                        .insert(readiness.username.clone(), readiness.clone());
                    self.apply_actions([GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)]);
                }
                GuiSemanticStep::ApplyMainWindowPlaylistSelection(index) => {
                    let mut snapshot = GuiInteractionRuntimeSnapshot::from_shell_state(&self.state);
                    snapshot.selection.selected_main_window_playlist = *index;
                    self.apply_actions([GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
                        snapshot,
                    )]);
                }
                GuiSemanticStep::ApplyPlayerSetupRuntimeSnapshot(snapshot) => {
                    self.apply_actions([GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
                        snapshot.clone(),
                    )]);
                }
                GuiSemanticStep::ApplySeekPreparationRuntimeSnapshot(snapshot) => {
                    self.apply_actions([GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
                        snapshot.clone(),
                    )]);
                }
                GuiSemanticStep::OpenMediaFiles(paths) => self.open_media_files(paths.clone())?,
                GuiSemanticStep::DropMediaFiles { target, paths } => {
                    self.drop_media_files(*target, paths.clone())?
                }
                GuiSemanticStep::AddMediaSearchDirectory(path) => {
                    if !self
                        .state
                        .apply(GuiShellAction::AddMediaSearchDirectory(path.clone()))
                    {
                        return Err(self
                            .state
                            .validation
                            .last_action_error
                            .clone()
                            .unwrap_or_else(|| {
                                "semantic media-search directory add failed".to_owned()
                            }));
                    }
                }
                GuiSemanticStep::PushChatMessage { sender, message } => {
                    self.apply_actions([GuiShellAction::PushChatMessage {
                        sender: sender.clone(),
                        message: message.clone(),
                    }]);
                }
                GuiSemanticStep::Activate(widget_id) => self.activate_widget(widget_id)?,
                GuiSemanticStep::EnterText {
                    widget_id,
                    value,
                    submit,
                } => self.enter_text(widget_id, value, *submit)?,
                GuiSemanticStep::AssertWidgetLabel { widget_id, label } => {
                    self.assert_widget_label(widget_id, label)?
                }
                GuiSemanticStep::AssertWidgetValue { widget_id, value } => {
                    self.assert_widget_value(widget_id, value.as_deref())?
                }
                GuiSemanticStep::AssertWidgetSelected {
                    widget_id,
                    selected,
                } => self.assert_widget_selected(widget_id, *selected)?,
                GuiSemanticStep::AssertWidgetEnabled { widget_id, enabled } => {
                    self.assert_widget_enabled(widget_id, *enabled)?
                }
                GuiSemanticStep::AssertPending { pending } => self.assert_pending(*pending)?,
                GuiSemanticStep::CompletePending => self.complete_pending()?,
                GuiSemanticStep::CompletePendingRuntime => self.complete_pending_via_runtime()?,
                GuiSemanticStep::CancelPending => self.cancel_pending()?,
                GuiSemanticStep::CloseModal => self.apply_actions([GuiShellAction::CloseModal]),
                GuiSemanticStep::ClearNotifications => {
                    self.apply_actions([GuiShellAction::ClearTransientNotifications])
                }
            }
        }
        Ok(())
    }
}

impl Drop for GuiSemanticDriver {
    fn drop(&mut self) {
        if let Some(root) = self.media_fixture_root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
