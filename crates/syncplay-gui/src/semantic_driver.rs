use super::{
    GuiDroppedFilesTarget, GuiInteractionRuntimeSnapshot, GuiPendingCompletionRequest,
    GuiPendingOperationKind, GuiPersistedConfigRuntimeOwner, GuiPreviewRuntimeBridge,
    GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiShellAction,
    GuiWidgetEguiRenderer, GuiWidgetKind, GuiWidgetNode, MainWindowRuntimeChatSnapshot,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    StoredClientSettingsMvp, SyncplayGuiShellAppState,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiSemanticStep {
    ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot),
    ApplyMainWindowPlaylistSelection(Option<usize>),
    OpenMediaFiles(Vec<String>),
    DropMediaFiles {
        target: GuiDroppedFilesTarget,
        paths: Vec<String>,
    },
    AddMediaSearchDirectory(String),
    PushChatMessage {
        sender: String,
        message: String,
    },
    Activate(String),
    EnterText {
        widget_id: String,
        value: String,
        submit: bool,
    },
    AssertWidgetLabel {
        widget_id: String,
        label: String,
    },
    AssertWidgetValue {
        widget_id: String,
        value: Option<String>,
    },
    AssertWidgetSelected {
        widget_id: String,
        selected: bool,
    },
    AssertWidgetEnabled {
        widget_id: String,
        enabled: bool,
    },
    AssertPending {
        pending: Option<GuiPendingOperationKind>,
    },
    CompletePending,
    CompletePendingRuntime,
    CancelPending,
    CloseModal,
    ClearNotifications,
}

impl GuiSemanticStep {
    pub(super) fn activate(widget_id: &str) -> Self {
        Self::Activate(widget_id.to_owned())
    }

    pub(super) fn enter_text(widget_id: &str, value: &str, submit: bool) -> Self {
        Self::EnterText {
            widget_id: widget_id.to_owned(),
            value: value.to_owned(),
            submit,
        }
    }

    pub(super) fn assert_widget_label(widget_id: &str, label: &str) -> Self {
        Self::AssertWidgetLabel {
            widget_id: widget_id.to_owned(),
            label: label.to_owned(),
        }
    }

    pub(super) fn assert_widget_value(widget_id: &str, value: Option<&str>) -> Self {
        Self::AssertWidgetValue {
            widget_id: widget_id.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    pub(super) fn assert_widget_selected(widget_id: &str, selected: bool) -> Self {
        Self::AssertWidgetSelected {
            widget_id: widget_id.to_owned(),
            selected,
        }
    }

    pub(super) fn assert_widget_enabled(widget_id: &str, enabled: bool) -> Self {
        Self::AssertWidgetEnabled {
            widget_id: widget_id.to_owned(),
            enabled,
        }
    }

    pub(super) fn assert_pending(pending: Option<GuiPendingOperationKind>) -> Self {
        Self::AssertPending { pending }
    }

    fn parse_index(token: &str) -> Result<usize, String> {
        token
            .parse::<usize>()
            .map_err(|_| format!("expected a non-negative index, got {token:?}"))
    }

    fn parse_optional_index(token: &str) -> Result<Option<usize>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_index(token).map(Some)
        }
    }

    fn parse_bool(token: &str) -> Result<bool, String> {
        match token {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("expected boolean 'true' or 'false', got {token:?}")),
        }
    }

    fn parse_drop_target(token: &str) -> Result<GuiDroppedFilesTarget, String> {
        GuiDroppedFilesTarget::parse(token)
    }

    fn parse_optional_value(token: &str) -> Option<&str> {
        match token {
            "<none>" => None,
            "<empty>" => Some(""),
            _ => Some(token),
        }
    }

    fn parse_pending(token: &str) -> Result<Option<GuiPendingOperationKind>, String> {
        let pending = match token {
            "none" => return Ok(None),
            "save-configuration" => GuiPendingOperationKind::SaveConfiguration,
            "reset-configuration" => GuiPendingOperationKind::ResetConfiguration,
            "reload-configuration" => GuiPendingOperationKind::ReloadConfiguration,
            "clear-gui-data" => GuiPendingOperationKind::ClearGuiData,
            "connect-saved-server" => GuiPendingOperationKind::ConnectSavedServer,
            "disconnect-session" => GuiPendingOperationKind::DisconnectSession,
            "connect-public-server" => GuiPendingOperationKind::ConnectPublicServer,
            "refresh-public-servers" => GuiPendingOperationKind::RefreshPublicServers,
            "search-missing-media" => GuiPendingOperationKind::SearchMissingMedia,
            "toggle-playback-pause" => GuiPendingOperationKind::TogglePlaybackPause,
            "send-chat-message" => GuiPendingOperationKind::SendChatMessage,
            _ => return Err(format!("unknown pending-operation label {token:?}")),
        };
        Ok(Some(pending))
    }

    fn split_list_token(token: &str) -> Vec<&str> {
        if token == "<none>" {
            Vec::new()
        } else {
            token.split('|').collect()
        }
    }

    fn parse_runtime_users(token: &str) -> Result<Vec<MainWindowRuntimeUserSnapshot>, String> {
        Self::split_list_token(token)
            .into_iter()
            .map(|entry| {
                let mut fields = entry.splitn(4, ',');
                let username = fields
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("runtime user entry {entry:?} is missing a username"))?;
                let is_self =
                    Self::parse_bool(fields.next().ok_or_else(|| {
                        format!("runtime user entry {entry:?} is missing is_self")
                    })?)?;
                let is_ready =
                    Self::parse_bool(fields.next().ok_or_else(|| {
                        format!("runtime user entry {entry:?} is missing is_ready")
                    })?)?;
                let is_controller = Self::parse_bool(fields.next().ok_or_else(|| {
                    format!("runtime user entry {entry:?} is missing is_controller")
                })?)?;
                Ok(MainWindowRuntimeUserSnapshot {
                    username: username.to_owned(),
                    room_name: String::new(),
                    is_self,
                    is_ready,
                    is_controller,
                    has_file: false,
                    file_name: None,
                    file_size_label: String::new(),
                    file_duration_label: String::new(),
                    file_is_url: false,
                    file_is_trusted: true,
                    filename_differs: false,
                    filesize_differs: false,
                    fileduration_differs: false,
                })
            })
            .collect()
    }

    fn parse_runtime_chat_rows(token: &str) -> Result<Vec<MainWindowRuntimeChatSnapshot>, String> {
        Self::split_list_token(token)
            .into_iter()
            .map(|entry| {
                let (sender, message) = entry.split_once('>').ok_or_else(|| {
                    format!("runtime chat entry {entry:?} must use 'sender>message' formatting")
                })?;
                if sender.is_empty() {
                    return Err(format!("runtime chat entry {entry:?} is missing a sender"));
                }
                Ok(MainWindowRuntimeChatSnapshot {
                    sender: sender.to_owned(),
                    message: message.to_owned(),
                })
            })
            .collect()
    }

    fn parse_main_window_runtime_snapshot<'a, I>(
        mut fields: I,
    ) -> Result<MainWindowRuntimeSnapshot, String>
    where
        I: Iterator<Item = &'a str>,
    {
        let room_name = fields
            .next()
            .ok_or_else(|| "apply-main-window-runtime requires a room name".to_owned())?
            .to_owned();
        let shared_playlist_enabled = Self::parse_bool(fields.next().ok_or_else(|| {
            "apply-main-window-runtime requires shared_playlist_enabled".to_owned()
        })?)?;
        let playback_paused = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires playback_paused".to_owned())?,
        )?;
        let autoplay_active = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires autoplay_active".to_owned())?,
        )?;
        let can_toggle_pause =
            Self::parse_bool(fields.next().ok_or_else(|| {
                "apply-main-window-runtime requires can_toggle_pause".to_owned()
            })?)?;
        let can_seek = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires can_seek".to_owned())?,
        )?;
        let can_set_ready = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires can_set_ready".to_owned())?,
        )?;
        let can_manage_playlist =
            Self::parse_bool(fields.next().ok_or_else(|| {
                "apply-main-window-runtime requires can_manage_playlist".to_owned()
            })?)?;
        let users = Self::parse_runtime_users(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires users".to_owned())?,
        )?
        .into_iter()
        .map(|mut user| {
            if user.room_name.is_empty() {
                user.room_name = room_name.clone();
            }
            user
        })
        .collect::<Vec<_>>();
        let playlist = Self::split_list_token(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires playlist".to_owned())?,
        )
        .into_iter()
        .map(str::to_owned)
        .collect();
        let chat = Self::parse_runtime_chat_rows(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires chat rows".to_owned())?,
        )?;
        if fields.next().is_some() {
            return Err("apply-main-window-runtime accepts exactly eleven arguments".to_owned());
        }
        let room_row_name = room_name.clone();
        Ok(MainWindowRuntimeSnapshot {
            room_name,
            shared_playlist_enabled,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: vec![MainWindowRuntimeRoomSnapshot {
                room_name: room_row_name,
                is_controlled: false,
                has_named_users: !users.is_empty(),
            }],
            users,
            playlist,
            chat,
            can_toggle_pause,
            can_seek,
            can_set_ready,
            can_manage_playlist,
            playback_paused,
            autoplay_active,
        })
    }

    fn from_script_line(line: &str) -> Result<Option<Self>, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(None);
        }

        let mut fields = trimmed.split('\t');
        let command = fields
            .next()
            .ok_or_else(|| "script line is missing a command".to_owned())?;

        let step =
            match command {
                "apply-main-window-runtime" => Self::ApplyMainWindowRuntimeSnapshot(
                    Self::parse_main_window_runtime_snapshot(fields)?,
                ),
                "apply-main-window-playlist-selection" => {
                    let index = Self::parse_optional_index(fields.next().ok_or_else(|| {
                        "apply-main-window-playlist-selection requires an index or 'none'"
                            .to_owned()
                    })?)?;
                    if fields.next().is_some() {
                        return Err(
                            "apply-main-window-playlist-selection accepts exactly one argument"
                                .to_owned(),
                        );
                    }
                    Self::ApplyMainWindowPlaylistSelection(index)
                }
                "push-chat-message" => {
                    let sender = fields
                        .next()
                        .ok_or_else(|| "push-chat-message requires a sender".to_owned())?;
                    let message = fields
                        .next()
                        .ok_or_else(|| "push-chat-message requires a message".to_owned())?;
                    if fields.next().is_some() {
                        return Err("push-chat-message accepts exactly two arguments".to_owned());
                    }
                    Self::PushChatMessage {
                        sender: sender.to_owned(),
                        message: message.to_owned(),
                    }
                }
                "open-media-files" => {
                    let entries =
                        Self::split_list_token(fields.next().ok_or_else(|| {
                            "open-media-files requires one or more paths".to_owned()
                        })?)
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    if fields.next().is_some() {
                        return Err("open-media-files accepts exactly one argument".to_owned());
                    }
                    if entries.is_empty() {
                        return Err("open-media-files requires at least one path".to_owned());
                    }
                    Self::OpenMediaFiles(entries)
                }
                "drop-media-files" => {
                    let target = Self::parse_drop_target(
                        fields
                            .next()
                            .ok_or_else(|| "drop-media-files requires a target".to_owned())?,
                    )?;
                    let entries =
                        Self::split_list_token(fields.next().ok_or_else(|| {
                            "drop-media-files requires one or more paths".to_owned()
                        })?)
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    if fields.next().is_some() {
                        return Err("drop-media-files accepts exactly two arguments".to_owned());
                    }
                    if entries.is_empty() {
                        return Err("drop-media-files requires at least one path".to_owned());
                    }
                    Self::DropMediaFiles {
                        target,
                        paths: entries,
                    }
                }
                "add-media-search-directory" => {
                    let path = fields
                        .next()
                        .ok_or_else(|| "add-media-search-directory requires a path".to_owned())?;
                    if fields.next().is_some() {
                        return Err(
                            "add-media-search-directory accepts exactly one argument".to_owned()
                        );
                    }
                    Self::AddMediaSearchDirectory(path.to_owned())
                }
                "activate" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "activate requires a widget id".to_owned())?;
                    if fields.next().is_some() {
                        return Err("activate accepts exactly one argument".to_owned());
                    }
                    Self::activate(widget_id)
                }
                "enter-text" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "enter-text requires a widget id".to_owned())?;
                    let submit = Self::parse_bool(
                        fields
                            .next()
                            .ok_or_else(|| "enter-text requires a submit flag".to_owned())?,
                    )?;
                    let value = fields
                        .next()
                        .ok_or_else(|| "enter-text requires a value".to_owned())?;
                    if fields.next().is_some() {
                        return Err("enter-text accepts exactly three arguments".to_owned());
                    }
                    Self::enter_text(widget_id, value, submit)
                }
                "assert-label" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "assert-label requires a widget id".to_owned())?;
                    let label = fields
                        .next()
                        .ok_or_else(|| "assert-label requires a label".to_owned())?;
                    if fields.next().is_some() {
                        return Err("assert-label accepts exactly two arguments".to_owned());
                    }
                    Self::assert_widget_label(widget_id, label)
                }
                "assert-value" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "assert-value requires a widget id".to_owned())?;
                    let value = fields
                        .next()
                        .ok_or_else(|| "assert-value requires a value token".to_owned())?;
                    if fields.next().is_some() {
                        return Err("assert-value accepts exactly two arguments".to_owned());
                    }
                    Self::assert_widget_value(widget_id, Self::parse_optional_value(value))
                }
                "assert-selected" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "assert-selected requires a widget id".to_owned())?;
                    let selected = Self::parse_bool(
                        fields
                            .next()
                            .ok_or_else(|| "assert-selected requires a boolean".to_owned())?,
                    )?;
                    if fields.next().is_some() {
                        return Err("assert-selected accepts exactly two arguments".to_owned());
                    }
                    Self::assert_widget_selected(widget_id, selected)
                }
                "assert-enabled" => {
                    let widget_id = fields
                        .next()
                        .ok_or_else(|| "assert-enabled requires a widget id".to_owned())?;
                    let enabled = Self::parse_bool(
                        fields
                            .next()
                            .ok_or_else(|| "assert-enabled requires a boolean".to_owned())?,
                    )?;
                    if fields.next().is_some() {
                        return Err("assert-enabled accepts exactly two arguments".to_owned());
                    }
                    Self::assert_widget_enabled(widget_id, enabled)
                }
                "assert-pending" => {
                    let pending =
                        Self::parse_pending(fields.next().ok_or_else(|| {
                            "assert-pending requires a pending label".to_owned()
                        })?)?;
                    if fields.next().is_some() {
                        return Err("assert-pending accepts exactly one argument".to_owned());
                    }
                    Self::assert_pending(pending)
                }
                "complete-pending" => {
                    if fields.next().is_some() {
                        return Err("complete-pending does not accept arguments".to_owned());
                    }
                    Self::CompletePending
                }
                "cancel-pending" => {
                    if fields.next().is_some() {
                        return Err("cancel-pending does not accept arguments".to_owned());
                    }
                    Self::CancelPending
                }
                "complete-pending-runtime" => {
                    if fields.next().is_some() {
                        return Err("complete-pending-runtime does not accept arguments".to_owned());
                    }
                    Self::CompletePendingRuntime
                }
                "close-modal" => {
                    if fields.next().is_some() {
                        return Err("close-modal does not accept arguments".to_owned());
                    }
                    Self::CloseModal
                }
                "clear-notifications" => {
                    if fields.next().is_some() {
                        return Err("clear-notifications does not accept arguments".to_owned());
                    }
                    Self::ClearNotifications
                }
                _ => return Err(format!("unknown semantic script command {command:?}")),
            };

        Ok(Some(step))
    }

    pub(super) fn parse_script(script: &str) -> Result<Vec<Self>, String> {
        script
            .lines()
            .enumerate()
            .filter_map(|(line_index, line)| match Self::from_script_line(line) {
                Ok(Some(step)) => Some(Ok(step)),
                Ok(None) => None,
                Err(error) => Some(Err(format!(
                    "semantic script line {} failed: {error}",
                    line_index + 1
                ))),
            })
            .collect()
    }
}

pub(super) struct GuiSemanticDriver {
    state: SyncplayGuiShellAppState,
}

impl GuiSemanticDriver {
    fn new(state: SyncplayGuiShellAppState) -> Self {
        Self { state }
    }

    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        Self::new(SyncplayGuiShellAppState::from_stored_settings(settings))
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> &SyncplayGuiShellAppState {
        &self.state
    }

    pub(super) fn active_view_label(&self) -> &'static str {
        self.state.active_view.label()
    }

    pub(super) fn active_modal_label(&self) -> &'static str {
        self.state
            .open_modal
            .map(|modal| modal.label())
            .unwrap_or("none")
    }

    pub(super) fn pending_operation_label(&self) -> &'static str {
        self.state
            .pending_operation
            .as_ref()
            .map(|operation| operation.kind.label())
            .unwrap_or("none")
    }

    pub(super) fn shell_tree(&self) -> GuiWidgetNode {
        self.state.shell_widget_tree()
    }

    pub(super) fn widget_count(&self) -> usize {
        self.shell_tree().node_count()
    }

    pub(super) fn widget(&self, widget_id: &str) -> Result<GuiWidgetNode, String> {
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

    fn activate_widget(&mut self, widget_id: &str) -> Result<(), String> {
        let widget = self.widget(widget_id)?;
        let actions = match widget.kind {
            GuiWidgetKind::Panel => GuiWidgetEguiRenderer::action_for_surface_node(&widget)
                .into_iter()
                .collect::<Vec<_>>(),
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
        self.apply_actions(actions);
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
        self.apply_actions(actions);
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
        let actions = GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            paths,
            self.state.shared_playlist_events_enabled(),
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
        let actions = GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            paths,
            target.load_into_shared_playlist(&self.state),
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

    fn complete_pending_via_runtime(&mut self) -> Result<(), String> {
        let Some(request) = GuiPendingCompletionRequest::from_state(&self.state) else {
            return Err(
                "semantic runtime completion requires an active pending operation".to_owned(),
            );
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(request));
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &self.state);
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

    fn run_steps(&mut self, steps: &[GuiSemanticStep]) -> Result<(), String> {
        for step in steps {
            match step {
                GuiSemanticStep::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                    self.apply_actions([GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                        snapshot.clone(),
                    )]);
                }
                GuiSemanticStep::ApplyMainWindowPlaylistSelection(index) => {
                    let mut snapshot = GuiInteractionRuntimeSnapshot::from_shell_state(&self.state);
                    snapshot.selection.selected_main_window_playlist = *index;
                    self.apply_actions([GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
                        snapshot,
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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiSemanticScenario {
    name: &'static str,
    initial_settings: StoredClientSettingsMvp,
    steps: Vec<GuiSemanticStep>,
}

impl GuiSemanticScenario {
    fn new(
        name: &'static str,
        initial_settings: StoredClientSettingsMvp,
        steps: Vec<GuiSemanticStep>,
    ) -> Self {
        Self {
            name,
            initial_settings,
            steps,
        }
    }

    pub(super) fn from_script(
        name: &'static str,
        initial_settings: StoredClientSettingsMvp,
        script: &str,
    ) -> Result<Self, String> {
        Ok(Self::new(
            name,
            initial_settings,
            GuiSemanticStep::parse_script(script)?,
        ))
    }

    pub(super) fn name(&self) -> &'static str {
        self.name
    }

    fn initial_settings(&self) -> &StoredClientSettingsMvp {
        &self.initial_settings
    }

    fn steps(&self) -> &[GuiSemanticStep] {
        &self.steps
    }

    pub(super) fn run(&self) -> Result<GuiSemanticDriver, String> {
        let mut driver = GuiSemanticDriver::from_stored_settings(self.initial_settings());
        driver.run_steps(self.steps())?;
        Ok(driver)
    }
}
