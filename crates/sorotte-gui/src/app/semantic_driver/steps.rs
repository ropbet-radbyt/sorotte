use super::super::{
    GuiDroppedFilesTarget, GuiPendingOperationKind, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, MainWindowRuntimeChatSnapshot, MainWindowRuntimeRoomSnapshot,
    MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiSemanticStep {
    ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot),
    ApplyMainWindowPlaylistSelection(Option<usize>),
    ApplyPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot),
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
    pub(in crate::app) fn activate(widget_id: &str) -> Self {
        Self::Activate(widget_id.to_owned())
    }

    pub(in crate::app) fn enter_text(widget_id: &str, value: &str, submit: bool) -> Self {
        Self::EnterText {
            widget_id: widget_id.to_owned(),
            value: value.to_owned(),
            submit,
        }
    }

    pub(in crate::app) fn assert_widget_label(widget_id: &str, label: &str) -> Self {
        Self::AssertWidgetLabel {
            widget_id: widget_id.to_owned(),
            label: label.to_owned(),
        }
    }

    pub(in crate::app) fn assert_widget_value(widget_id: &str, value: Option<&str>) -> Self {
        Self::AssertWidgetValue {
            widget_id: widget_id.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    pub(in crate::app) fn assert_widget_selected(widget_id: &str, selected: bool) -> Self {
        Self::AssertWidgetSelected {
            widget_id: widget_id.to_owned(),
            selected,
        }
    }

    pub(in crate::app) fn assert_widget_enabled(widget_id: &str, enabled: bool) -> Self {
        Self::AssertWidgetEnabled {
            widget_id: widget_id.to_owned(),
            enabled,
        }
    }

    pub(in crate::app) fn assert_pending(pending: Option<GuiPendingOperationKind>) -> Self {
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

    fn decode_text_token(token: &str) -> String {
        let mut decoded = String::new();
        let mut characters = token.chars();
        while let Some(character) = characters.next() {
            if character != '\\' {
                decoded.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => decoded.push('\n'),
                Some('r') => decoded.push('\r'),
                Some('t') => decoded.push('\t'),
                Some('\\') => decoded.push('\\'),
                Some(other) => {
                    decoded.push('\\');
                    decoded.push(other);
                }
                None => decoded.push('\\'),
            }
        }
        decoded
    }

    fn parse_optional_value(token: &str) -> Option<String> {
        match token {
            "<none>" => None,
            "<empty>" => Some(String::new()),
            _ => Some(Self::decode_text_token(token)),
        }
    }

    fn parse_pending(token: &str) -> Result<Option<GuiPendingOperationKind>, String> {
        let pending = match token {
            "none" => return Ok(None),
            "save-configuration" => GuiPendingOperationKind::SaveConfiguration,
            "reset-configuration" => GuiPendingOperationKind::ResetConfiguration,
            "reload-configuration" => GuiPendingOperationKind::ReloadConfiguration,
            "clear-gui-data" => GuiPendingOperationKind::ClearGuiData,
            "change-config-storage-root" => GuiPendingOperationKind::ChangeConfigStorageRoot,
            "connect-saved-server" => GuiPendingOperationKind::ConnectSavedServer,
            "disconnect-session" => GuiPendingOperationKind::DisconnectSession,
            "connect-public-server" => GuiPendingOperationKind::ConnectPublicServer,
            "refresh-public-servers" => GuiPendingOperationKind::RefreshPublicServers,
            "search-missing-media" => GuiPendingOperationKind::SearchMissingMedia,
            "pause-playback" => GuiPendingOperationKind::SetPlaybackPause(true),
            "resume-playback" => GuiPendingOperationKind::SetPlaybackPause(false),
            "toggle-playback-pause" => GuiPendingOperationKind::TogglePlaybackPause,
            "send-chat-message" => GuiPendingOperationKind::SendChatMessage,
            _ => return Err(format!("unknown pending-operation label {token:?}")),
        };
        Ok(Some(pending))
    }

    fn parse_player_setup_issue_kind(token: &str) -> Result<GuiPlayerSetupIssueKind, String> {
        match token {
            "not-configured" => Ok(GuiPlayerSetupIssueKind::NotConfigured),
            "unsupported-player" => Ok(GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer),
            "missing-binary" => Ok(GuiPlayerSetupIssueKind::MissingBinary),
            "launch-failed" => Ok(GuiPlayerSetupIssueKind::LaunchFailed),
            "ipc-attach-failed" => Ok(GuiPlayerSetupIssueKind::IpcAttachFailed),
            "exited-after-launch" => Ok(GuiPlayerSetupIssueKind::ExitedAfterLaunch),
            _ => Err(format!("unknown player-setup issue label {token:?}")),
        }
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
            ..Default::default()
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
                "apply-player-setup-runtime" => {
                    let kind =
                        Self::parse_player_setup_issue_kind(fields.next().ok_or_else(|| {
                            "apply-player-setup-runtime requires an issue kind".to_owned()
                        })?)?;
                    let message = Self::decode_text_token(fields.next().ok_or_else(|| {
                        "apply-player-setup-runtime requires an issue message".to_owned()
                    })?);
                    if fields.next().is_some() {
                        return Err(
                            "apply-player-setup-runtime accepts exactly two arguments".to_owned()
                        );
                    }
                    Self::ApplyPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot {
                        issue: Some(GuiPlayerSetupIssue { kind, message }),
                    })
                }
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
                    Self::enter_text(widget_id, &Self::decode_text_token(value), submit)
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
                    let parsed_value = Self::parse_optional_value(value);
                    Self::assert_widget_value(widget_id, parsed_value.as_deref())
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

    pub(in crate::app) fn parse_script(script: &str) -> Result<Vec<Self>, String> {
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
