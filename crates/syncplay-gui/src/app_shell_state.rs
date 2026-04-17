use syncplay_client_app::app_boundary::{
    compatibility::{
        LegacyConfigurationGetterCompatibilityStatus,
        legacy_configuration_getter_startup_compat_entries,
    },
    language::{
        SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
        normalized_legacy_runtime_language_tag_legacy_compatible,
    },
    state::{
        StoredClientSettingsMvp, autoplay_threshold_override_legacy_value_compatible,
        privacy_mode_legacy_name_compatible, unpause_action_mode_legacy_name_compatible,
    },
};

use super::GuiLaunchMode;
use super::remote_services;
use super::support::{
    bool_label, optional_f64_text, optional_i64_text, optional_port_text, optional_room_text,
    optional_string_list_multiline_text, optional_text, player_arguments_text_for_path,
};
use super::ui_state::GuiUpdateCheckState;
use super::widget_tree::GuiWidgetKind;

#[cfg(test)]
#[path = "app_shell_state/tests.rs"]
mod tests;

#[path = "app_shell_state/browser_support.rs"]
mod browser_support;
#[path = "app_shell_state/configuration_dialog.rs"]
mod configuration_dialog;
#[path = "app_shell_state/main_window.rs"]
mod main_window;

pub(super) use self::browser_support::{
    browser_domain_from_url, browser_format_duration_label, browser_format_size_label,
    browser_is_url, browser_uri_is_trusted, load_playlist_entries_from_path,
    playlist_entries_from_multiline_text, playlist_entries_multiline_text,
    save_playlist_entries_to_path, shuffle_playlist_entries_in_place,
};
pub(super) use self::configuration_dialog::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, GuiChatSection,
    GuiConnectionSettingsSection, GuiDesyncSection, GuiDialogControl, GuiDialogControlKind,
    GuiDialogSection, GuiMediaSearchSection, GuiOsdSection, GuiPrivacySection, GuiReadinessSection,
    GuiSystemSection,
};
pub(super) use self::main_window::{
    MainWindowChatRow, MainWindowPlaybackControls, MainWindowPlaylistRow, MainWindowRoomRow,
    MainWindowRuntimeChatSnapshot, MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot,
    MainWindowRuntimeUserSnapshot, MainWindowShellState, MainWindowUserRow,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuActionShellItem {
    pub(super) label: &'static str,
    pub(super) enabled: bool,
    pub(super) is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuSectionShellState {
    pub(super) title: &'static str,
    pub(super) actions: Vec<MenuActionShellItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuDialogShellState {
    pub(super) sections: Vec<MenuSectionShellState>,
    pub(super) tls_prompt_expected: bool,
    pub(super) update_notice_expected: bool,
    pub(super) about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuActionRuntimeOverride {
    pub(super) section_title: &'static str,
    pub(super) action_label: &'static str,
    pub(super) enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuDialogRuntimeSnapshot {
    pub(super) action_overrides: Vec<MenuActionRuntimeOverride>,
    pub(super) tls_prompt_expected: bool,
    pub(super) update_notice_expected: bool,
    pub(super) about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicServerBrowserRow {
    pub(super) label: String,
    pub(super) address: String,
    pub(super) is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicServerBrowserShellState {
    pub(super) servers: Vec<PublicServerBrowserRow>,
    pub(super) can_connect: bool,
    pub(super) can_refresh: bool,
    pub(super) can_add_custom_server: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiMediaIndexStatusState {
    pub(super) active: bool,
    pub(super) message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiMediaIndexRuntimeSnapshot {
    pub(super) active: bool,
    pub(super) message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPlayerSetupIssueKind {
    NotConfigured,
    UnsupportedConfiguredPlayer,
    MissingBinary,
    LaunchFailed,
    IpcAttachFailed,
    ExitedAfterLaunch,
}

impl GuiPlayerSetupIssueKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::UnsupportedConfiguredPlayer => "unsupported-player",
            Self::MissingBinary => "missing-binary",
            Self::LaunchFailed => "launch-failed",
            Self::IpcAttachFailed => "ipc-attach-failed",
            Self::ExitedAfterLaunch => "exited-after-launch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlayerSetupIssue {
    pub(super) kind: GuiPlayerSetupIssueKind,
    pub(super) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiPlayerSetupRuntimeSnapshot {
    pub(super) issue: Option<GuiPlayerSetupIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PublicServerBrowserRuntimeFlags {
    pub(super) can_connect: bool,
    pub(super) can_refresh: bool,
    pub(super) can_add_custom_server: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MediaSearchDirectoryRow {
    pub(super) path: String,
    pub(super) is_selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MediaSearchWorkflowShellState {
    pub(super) directories: Vec<MediaSearchDirectoryRow>,
    pub(super) can_browse_directories: bool,
    pub(super) can_search_missing_media: bool,
    pub(super) first_file_timeout_seconds: Option<f64>,
    pub(super) search_timeout_seconds: Option<f64>,
    pub(super) double_check_interval_seconds: Option<f64>,
    pub(super) warning_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MediaSearchWorkflowRuntimeFlags {
    pub(super) can_browse_directories: bool,
    pub(super) can_search_missing_media: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SyncplayGuiRuntimeSnapshot {
    pub(super) active_view: GuiShellView,
    pub(super) open_modal: Option<GuiShellModal>,
    pub(super) main_window: MainWindowRuntimeSnapshot,
    pub(super) public_servers: PublicServerBrowserShellState,
    pub(super) media_search: MediaSearchWorkflowShellState,
    pub(super) tls_prompt_expected: bool,
    pub(super) update_notice_expected: bool,
    pub(super) about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiFeedbackRuntimeSnapshot {
    pub(super) validation_issues: Vec<GuiValidationIssue>,
    pub(super) notifications: Vec<GuiTransientNotification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiErrorRuntimeSnapshot {
    pub(super) last_action_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SyncplayGuiShellAppState {
    pub(super) active_view: GuiShellView,
    pub(super) selected_main_window_tab: GuiMainWindowTab,
    pub(super) selected_configuration_tab: GuiConfigurationTab,
    pub(super) open_modal: Option<GuiShellModal>,
    pub(super) selection: GuiSelectionState,
    pub(super) main_window_playlist_selection_is_local: bool,
    pub(super) runtime_menu_action_overrides: Vec<MenuActionRuntimeOverride>,
    pub(super) runtime_command_availability_override: GuiCommandAvailabilityRuntimeOverride,
    pub(super) commands: GuiCommandAvailabilityState,
    pub(super) pending_operation: Option<GuiPendingOperationState>,
    pub(super) pending_local_ready_target: Option<bool>,
    pub(super) pending_saved_server_connect_saves_configuration: bool,
    pub(super) outgoing_chat_message: Option<String>,
    pub(super) new_main_window_user_draft: String,
    pub(super) new_playlist_entry_draft: String,
    pub(super) focused_configuration_control: Option<GuiFocusedConfigurationControlState>,
    pub(super) public_server_edit_session: Option<GuiPublicServerEditSessionState>,
    pub(super) main_window_user_edit_session: Option<GuiMainWindowUserEditSessionState>,
    pub(super) text_edit_session: Option<GuiTextEditSessionState>,
    pub(super) playlist_text_edit_session: Option<GuiPlaylistTextEditSessionState>,
    pub(super) playlist_url_edit_session: Option<GuiUrlEditSessionState>,
    pub(super) media_url_edit_session: Option<GuiUrlEditSessionState>,
    pub(super) controlled_room_create_session: Option<GuiControlledRoomCreateSessionState>,
    pub(super) controller_auth_edit_session: Option<GuiControllerAuthEditSessionState>,
    pub(super) room_history_edit_session: Option<GuiRoomHistoryEditSessionState>,
    pub(super) update_check: GuiUpdateCheckState,
    pub(super) runtime_validation_issues: Vec<GuiValidationIssue>,
    pub(super) notifications: Vec<GuiTransientNotification>,
    pub(super) validation: GuiValidationState,
    pub(super) last_media_dialog_directory: Option<String>,
    pub(super) playlist_undo_snapshot: Option<Vec<String>>,
    pub(super) playlist_shuffle_nonce: u64,
    pub(super) media_index_status: GuiMediaIndexStatusState,
    pub(super) player_setup_issue: Option<GuiPlayerSetupIssue>,
    pub(super) saved_configuration: StoredClientSettingsMvp,
    pub(super) configuration: FirstRunConfigurationDialogDraft,
    pub(super) main_window: MainWindowShellState,
    pub(super) menus: MenuDialogShellState,
    pub(super) public_servers: PublicServerBrowserShellState,
    pub(super) media_search: MediaSearchWorkflowShellState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiSelectionState {
    pub(super) selected_main_window_user: Option<usize>,
    pub(super) selected_main_window_playlist: Option<usize>,
    pub(super) selected_menu_action: Option<(usize, usize)>,
    pub(super) selected_media_search_directory: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiCommandAvailabilityState {
    pub(super) can_save_configuration: bool,
    pub(super) can_reset_configuration: bool,
    pub(super) can_reload_configuration: bool,
    pub(super) can_connect_saved_server: bool,
    pub(super) can_disconnect_session: bool,
    pub(super) can_connect_public_server: bool,
    pub(super) can_refresh_public_servers: bool,
    pub(super) can_search_missing_media: bool,
    pub(super) can_toggle_pause: bool,
    pub(super) can_send_chat_message: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiCommandAvailabilityRuntimeOverride {
    pub(super) can_save_configuration: Option<bool>,
    pub(super) can_reset_configuration: Option<bool>,
    pub(super) can_reload_configuration: Option<bool>,
    pub(super) can_connect_saved_server: Option<bool>,
    pub(super) can_disconnect_session: Option<bool>,
    pub(super) can_connect_public_server: Option<bool>,
    pub(super) can_refresh_public_servers: Option<bool>,
    pub(super) can_search_missing_media: Option<bool>,
    pub(super) can_toggle_pause: Option<bool>,
    pub(super) can_send_chat_message: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiCommandRuntimeSnapshot {
    pub(super) command_availability: GuiCommandAvailabilityState,
    pub(super) pending_operation: Option<GuiPendingOperationKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiFocusedConfigurationControlRuntimeSnapshot {
    pub(super) section: String,
    pub(super) label: String,
    pub(super) activation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPublicServerEditSessionRuntimeSnapshot {
    pub(super) editing_index: Option<usize>,
    pub(super) label_buffer: String,
    pub(super) address_buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiMainWindowUserEditSessionRuntimeSnapshot {
    pub(super) editing_index: usize,
    pub(super) username_buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiTextEditSessionRuntimeSnapshot {
    pub(super) section: String,
    pub(super) label: String,
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlaylistTextEditSessionRuntimeSnapshot {
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiUrlEditSessionRuntimeSnapshot {
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiInteractionRuntimeSnapshot {
    pub(super) selection: GuiSelectionState,
    pub(super) selected_public_server_index: Option<usize>,
    pub(super) focused_configuration_control: Option<GuiFocusedConfigurationControlRuntimeSnapshot>,
    pub(super) public_server_edit_session: Option<GuiPublicServerEditSessionRuntimeSnapshot>,
    pub(super) main_window_user_edit_session: Option<GuiMainWindowUserEditSessionRuntimeSnapshot>,
    pub(super) text_edit_session: Option<GuiTextEditSessionRuntimeSnapshot>,
    pub(super) playlist_text_edit_session: Option<GuiPlaylistTextEditSessionRuntimeSnapshot>,
    pub(super) playlist_url_edit_session: Option<GuiUrlEditSessionRuntimeSnapshot>,
    pub(super) media_url_edit_session: Option<GuiUrlEditSessionRuntimeSnapshot>,
}

impl GuiInteractionRuntimeSnapshot {
    pub(super) fn from_shell_state(state: &SyncplayGuiShellAppState) -> Self {
        Self {
            selection: state.selection.clone(),
            selected_public_server_index: state.selected_public_server_index(),
            focused_configuration_control: state.focused_configuration_control.as_ref().map(
                |focused| GuiFocusedConfigurationControlRuntimeSnapshot {
                    section: focused.section.to_owned(),
                    label: focused.label.to_owned(),
                    activation_count: focused.activation_count,
                },
            ),
            public_server_edit_session: state.public_server_edit_session.as_ref().map(|session| {
                GuiPublicServerEditSessionRuntimeSnapshot {
                    editing_index: session.editing_index,
                    label_buffer: session.label_buffer.clone(),
                    address_buffer: session.address_buffer.clone(),
                    is_dirty: session.is_dirty,
                }
            }),
            main_window_user_edit_session: state.main_window_user_edit_session.as_ref().map(
                |session| GuiMainWindowUserEditSessionRuntimeSnapshot {
                    editing_index: session.editing_index,
                    username_buffer: session.username_buffer.clone(),
                    is_dirty: session.is_dirty,
                },
            ),
            text_edit_session: state.text_edit_session.as_ref().map(|session| {
                GuiTextEditSessionRuntimeSnapshot {
                    section: session.section.to_owned(),
                    label: session.label.to_owned(),
                    buffer: session.buffer.clone(),
                    is_dirty: session.is_dirty,
                }
            }),
            playlist_text_edit_session: state.playlist_text_edit_session.as_ref().map(|session| {
                GuiPlaylistTextEditSessionRuntimeSnapshot {
                    buffer: session.buffer.clone(),
                    is_dirty: session.is_dirty,
                }
            }),
            playlist_url_edit_session: state.playlist_url_edit_session.as_ref().map(|session| {
                GuiUrlEditSessionRuntimeSnapshot {
                    buffer: session.buffer.clone(),
                    is_dirty: session.is_dirty,
                }
            }),
            media_url_edit_session: state.media_url_edit_session.as_ref().map(|session| {
                GuiUrlEditSessionRuntimeSnapshot {
                    buffer: session.buffer.clone(),
                    is_dirty: session.is_dirty,
                }
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiDraftRuntimeSnapshot {
    pub(super) outgoing_chat_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiConfigurationDraftRuntimeSnapshot {
    pub(super) settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiSavedConfigurationRuntimeSnapshot {
    pub(super) settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiConfigurationRuntimeSnapshot {
    pub(super) draft_settings: StoredClientSettingsMvp,
    pub(super) saved_settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiSavedSessionConnectTarget {
    pub(super) address: String,
    pub(super) username: String,
    pub(super) room: String,
    pub(super) controlled_room_password_override: Option<String>,
}

impl GuiDialogControlKind {
    pub(super) fn widget_kind(self) -> GuiWidgetKind {
        match self {
            Self::TextInput => GuiWidgetKind::TextInput,
            Self::TextArea => GuiWidgetKind::TextArea,
            Self::PasswordInput => GuiWidgetKind::PasswordInput,
            Self::Checkbox => GuiWidgetKind::Checkbox,
            Self::Select => GuiWidgetKind::Select,
            Self::NumericInput => GuiWidgetKind::NumericInput,
            Self::ReadOnly => GuiWidgetKind::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPendingOperationKind {
    SaveConfiguration,
    ResetConfiguration,
    ReloadConfiguration,
    ClearGuiData,
    ConnectSavedServer,
    DisconnectSession,
    ConnectPublicServer,
    RefreshPublicServers,
    SearchMissingMedia,
    TogglePlaybackPause,
    SendChatMessage,
}

impl GuiPendingOperationKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::SaveConfiguration => "save-configuration",
            Self::ResetConfiguration => "reset-configuration",
            Self::ReloadConfiguration => "reload-configuration",
            Self::ClearGuiData => "clear-gui-data",
            Self::ConnectSavedServer => "connect-saved-server",
            Self::DisconnectSession => "disconnect-session",
            Self::ConnectPublicServer => "connect-public-server",
            Self::RefreshPublicServers => "refresh-public-servers",
            Self::SearchMissingMedia => "search-missing-media",
            Self::TogglePlaybackPause => "toggle-playback-pause",
            Self::SendChatMessage => "send-chat-message",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPendingOperationState {
    pub(super) kind: GuiPendingOperationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiFocusedConfigurationControlState {
    pub(super) section: &'static str,
    pub(super) label: &'static str,
    pub(super) kind: GuiDialogControlKind,
    pub(super) activation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPublicServerEditSessionState {
    pub(super) editing_index: Option<usize>,
    pub(super) label_buffer: String,
    pub(super) address_buffer: String,
    pub(super) is_dirty: bool,
    pub(super) original_label: Option<String>,
    pub(super) original_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiMainWindowUserEditSessionState {
    pub(super) editing_index: usize,
    pub(super) username_buffer: String,
    pub(super) is_dirty: bool,
    pub(super) original_username: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiTextEditSessionState {
    pub(super) section: &'static str,
    pub(super) label: &'static str,
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlaylistTextEditSessionState {
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiUrlEditSessionState {
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiControlledRoomCreateSessionState {
    pub(super) room_buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiControllerAuthEditSessionState {
    pub(super) room_name: String,
    pub(super) password_buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiRoomHistoryEditSessionState {
    pub(super) buffer: String,
    pub(super) is_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiTransientNotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl GuiTransientNotificationLevel {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiTransientNotification {
    pub(super) level: GuiTransientNotificationLevel,
    pub(super) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiValidationIssue {
    pub(super) scope: String,
    pub(super) label: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiValidationState {
    pub(super) issues: Vec<GuiValidationIssue>,
    pub(super) last_action_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum GuiMainWindowTab {
    #[default]
    Overview,
    Session,
    Playback,
    Playlist,
    Chat,
}

impl GuiMainWindowTab {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Session => "session",
            Self::Playback => "playback",
            Self::Playlist => "playlist",
            Self::Chat => "chat",
        }
    }

    pub(super) fn from_label(label: &str) -> Option<Self> {
        match label {
            "overview" => Some(Self::Overview),
            "session" => Some(Self::Session),
            "playback" => Some(Self::Playback),
            "playlist" => Some(Self::Playlist),
            "chat" => Some(Self::Chat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum GuiConfigurationTab {
    #[default]
    Overview,
    Connection,
    PlaybackSearch,
    PrivacyChat,
    InterfaceSystem,
}

impl GuiConfigurationTab {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Connection => "connection",
            Self::PlaybackSearch => "playback-search",
            Self::PrivacyChat => "privacy-chat",
            Self::InterfaceSystem => "interface-system",
        }
    }

    pub(super) fn from_label(label: &str) -> Option<Self> {
        match label {
            "overview" => Some(Self::Overview),
            "connection" => Some(Self::Connection),
            "playback-search" => Some(Self::PlaybackSearch),
            "privacy-chat" => Some(Self::PrivacyChat),
            "interface-system" => Some(Self::InterfaceSystem),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiShellView {
    Configuration,
    MainWindow,
    MenusAndDialogs,
    PublicServers,
    MediaSearch,
}

impl GuiShellView {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::MainWindow => "main-window",
            Self::MenusAndDialogs => "menus-and-dialogs",
            Self::PublicServers => "public-servers",
            Self::MediaSearch => "media-search",
        }
    }

    pub(super) fn from_label(label: &str) -> Option<Self> {
        match label {
            "configuration" => Some(Self::Configuration),
            "main-window" => Some(Self::MainWindow),
            "menus-and-dialogs" => Some(Self::MenusAndDialogs),
            "public-servers" => Some(Self::PublicServers),
            "media-search" => Some(Self::MediaSearch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiShellModal {
    About,
    UpdateNotice,
    TlsCertificatePrompt,
    PlayerSetup,
}

impl GuiShellModal {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::UpdateNotice => "update-notice",
            Self::TlsCertificatePrompt => "tls-certificate-prompt",
            Self::PlayerSetup => "player-setup",
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiShellAction {
    SwitchView(GuiShellView),
    SelectMainWindowTab(GuiMainWindowTab),
    SelectConfigurationTab(GuiConfigurationTab),
    OpenModal(GuiShellModal),
    CloseModal,
    DismissUpdateNotice,
    BeginUpdateCheck {
        user_initiated: bool,
    },
    ApplyUpdateCheckResult(remote_services::LegacyUpdateCheckResult),
    ApplyStartupPublicServerCache(Vec<(String, String)>),
    TrustTlsCertificatePrompt,
    RejectTlsCertificatePrompt,
    TriggerSelectedMenuAction,
    AnnounceTlsCertificatePromptRequired,
    AnnounceUpdateNoticeAvailable,
    AnnounceAboutDialogRequested,
    AnnounceHelpRequested,
    ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot),
    ApplyGuiFeedbackRuntimeSnapshot(GuiFeedbackRuntimeSnapshot),
    ApplyGuiErrorRuntimeSnapshot(GuiErrorRuntimeSnapshot),
    ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot),
    ApplyGuiMediaIndexRuntimeSnapshot(GuiMediaIndexRuntimeSnapshot),
    ApplyGuiPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot),
    ApplyGuiInteractionRuntimeSnapshot(GuiInteractionRuntimeSnapshot),
    ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot),
    ApplyGuiConfigurationDraftRuntimeSnapshot(GuiConfigurationDraftRuntimeSnapshot),
    ApplyGuiSavedConfigurationRuntimeSnapshot(GuiSavedConfigurationRuntimeSnapshot),
    ApplyGuiConfigurationRuntimeSnapshot(GuiConfigurationRuntimeSnapshot),
    BeginConfigurationSave,
    CompleteConfigurationSave(StoredClientSettingsMvp),
    CancelConfigurationSave,
    BeginConfigurationReset,
    CompleteConfigurationReset(StoredClientSettingsMvp),
    CancelConfigurationReset,
    BeginConfigurationReload,
    CompleteConfigurationReload(StoredClientSettingsMvp),
    CancelConfigurationReload,
    BeginClearGuiData,
    CompleteClearGuiData,
    CancelClearGuiData,
    BeginPendingOperation(GuiPendingOperationKind),
    CompletePendingOperation,
    CancelPendingOperation,
    FocusConfigurationControl {
        section: &'static str,
        label: &'static str,
    },
    ActivateFocusedConfigurationControl,
    ClearConfigurationControlFocus,
    BeginAddPublicServer,
    BeginEditSelectedPublicServer,
    UpdatePublicServerEditLabel(String),
    UpdatePublicServerEditAddress(String),
    CommitPublicServerEdit,
    CancelPublicServerEdit,
    RemoveSelectedPublicServer,
    BeginEditSelectedMainWindowUser,
    UpdateMainWindowUserEdit(String),
    CommitMainWindowUserEdit,
    CancelMainWindowUserEdit,
    PushTransientNotification {
        level: GuiTransientNotificationLevel,
        message: String,
    },
    DismissTransientNotification(usize),
    ClearTransientNotifications,
    BeginConfigurationTextEdit {
        section: &'static str,
        label: &'static str,
    },
    UpdateConfigurationTextEdit(String),
    CommitConfigurationTextEdit,
    CancelConfigurationTextEdit,
    BeginRoomHistoryEdit,
    UpdateRoomHistoryEdit(String),
    CommitRoomHistoryEdit,
    CancelRoomHistoryEdit,
    BeginSharedPlaylistTextEdit,
    UpdateSharedPlaylistTextEdit(String),
    CancelSharedPlaylistTextEdit,
    BeginSharedPlaylistUrlEdit,
    UpdateSharedPlaylistUrlEdit(String),
    CancelSharedPlaylistUrlEdit,
    BeginMediaUrlEdit,
    UpdateMediaUrlEdit(String),
    CancelMediaUrlEdit,
    BeginCreateControlledRoomEdit,
    UpdateCreateControlledRoomEdit(String),
    CancelCreateControlledRoomEdit,
    BeginControllerAuthEdit,
    UpdateControllerAuthPasswordEdit(String),
    CancelControllerAuthEdit,
    UpdateNewMainWindowUserDraft(String),
    CommitNewMainWindowUser,
    UpdateNewPlaylistEntryDraft(String),
    CommitNewPlaylistEntry,
    AppendSharedPlaylistEntries(Vec<String>),
    ReplaceSharedPlaylistEntries(Vec<String>),
    LoadSharedPlaylistFromFile {
        path: String,
        entries: Vec<String>,
        shuffled: bool,
    },
    SaveSharedPlaylistToFile(String),
    SelectMainWindowUser(usize),
    AddMainWindowUser(String),
    AnnounceMainWindowUserJoined(String),
    AnnounceSelectedMainWindowUserRenamed(String),
    AnnounceSelectedMainWindowUserLeft,
    BeginPlaybackPause,
    BeginPlaybackResume,
    BeginPlaybackPauseToggle,
    CompletePlaybackPauseToggle,
    CancelPlaybackPauseToggle,
    AnnouncePlaybackPaused,
    AnnouncePlaybackResumed,
    RequestSeekPrompt,
    RequestOffsetPrompt,
    RequestPlaybackUndoSeek,
    AnnounceLocalUserReady,
    AnnounceLocalUserNotReady,
    AnnounceAutoplayState(bool),
    AnnounceAutoplayThreshold(usize),
    AnnounceSharedPlaylistLoaded(Vec<String>),
    AnnounceSharedPlaylistEntryAdded(String),
    AnnounceSharedPlaylistSelectionChanged(usize),
    AnnounceSelectedSharedPlaylistEntryRemoved,
    UndoSharedPlaylistChange,
    ShuffleRemainingSharedPlaylist,
    ShuffleEntireSharedPlaylist,
    BeginLocalChatSend(String),
    CompleteLocalChatSend,
    CancelLocalChatSend,
    AnnounceRemoteChatMessage {
        sender: String,
        message: String,
    },
    AnnounceSystemChatEvent(String),
    ToggleSelectedMainWindowUserReady,
    ToggleSelectedMainWindowUserController,
    RemoveSelectedMainWindowUser,
    SelectMainWindowPlaylist(usize),
    ActivateMainWindowPlaylist(usize),
    MoveMainWindowPlaylistRow {
        from_index: usize,
        to_index: usize,
    },
    MoveSelectedMainWindowPlaylistUp,
    MoveSelectedMainWindowPlaylistDown,
    RemoveSelectedMainWindowPlaylist,
    SelectMenuAction {
        section_index: usize,
        action_index: usize,
    },
    SelectMediaSearchDirectory(usize),
    MoveSelectedMediaSearchDirectoryUp,
    MoveSelectedMediaSearchDirectoryDown,
    RemoveSelectedMediaSearchDirectory,
    EditConfigurationText {
        section: &'static str,
        label: &'static str,
        value: String,
    },
    EditConfigurationBool {
        section: &'static str,
        label: &'static str,
        value: bool,
    },
    AnnouncePublicServerSelectionChanged(usize),
    BeginSavedServerConnect,
    CompleteSavedServerConnect,
    CancelSavedServerConnect,
    BeginSessionDisconnect,
    CompleteSessionDisconnect,
    CancelSessionDisconnect,
    BeginSelectedPublicServerConnect,
    CompleteSelectedPublicServerConnect,
    BeginPublicServerRefresh,
    CompletePublicServerRefresh(Vec<(String, String)>),
    AnnounceCustomPublicServerAdded {
        label: String,
        address: String,
    },
    SelectPublicServer(usize),
    AddMediaSearchDirectory(String),
    AnnounceMediaSearchDirectorySelected(usize),
    AnnounceMediaSearchDirectoryBrowsed(String),
    BeginMissingMediaSearch,
    CompleteMissingMediaSearch(Option<String>),
    RetryPlayerLaunch,
    ToggleMainWindowPlaybackButtons,
    ToggleMainWindowAutoplayControls,
    ToggleMainWindowHideEmptyRooms,
    RequestMainWindowUserMediaOpen(String),
    RequestMainWindowUserContainingFolderOpen(String),
    RequestMainWindowUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    AddTrustedDomain(String),
    JoinMainWindowRoom(String),
    LeaveMainWindowRoom,
    SetMainWindowRoom(String),
    ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot),
    ApplyGuiRuntimeSnapshot(SyncplayGuiRuntimeSnapshot),
    PushChatMessage {
        sender: String,
        message: String,
    },
}

impl FirstRunConfigurationDialogState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let startup_entries = legacy_configuration_getter_startup_compat_entries();
        let ignored_startup_exception_count = startup_entries
            .iter()
            .filter(|entry| entry.status == LegacyConfigurationGetterCompatibilityStatus::Ignored)
            .count();

        Self {
            launch_mode: if settings == &StoredClientSettingsMvp::default() {
                GuiLaunchMode::FirstRun
            } else {
                GuiLaunchMode::ExistingConfig
            },
            connection: GuiConnectionSettingsSection {
                host: settings.host.clone(),
                port: settings.port,
                username: settings.username.clone(),
                room: settings.room.clone(),
                server_password_set: settings
                    .server_password
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty()),
                player_path: settings.player_path.clone(),
                player_arguments_text: player_arguments_text_for_path(
                    settings.per_player_arguments.as_ref(),
                    settings.player_path.as_deref(),
                ),
                room_history_text: optional_string_list_multiline_text(
                    settings.room_list.as_deref(),
                ),
                public_server_count: settings.public_servers.as_ref().map_or(0, Vec::len),
                room_history_count: settings.room_list.as_ref().map_or(0, Vec::len),
            },
            readiness: GuiReadinessSection {
                ready_at_start: settings.ready_at_start.unwrap_or(false),
                autoplay_enabled: settings.autoplay_initial_state.unwrap_or(false),
                autoplay_require_same_filenames: settings
                    .autoplay_require_same_filenames
                    .unwrap_or(false),
                shared_playlist_enabled: settings.shared_playlist_enabled.unwrap_or(false),
                pause_on_leave: settings.pause_on_leave.unwrap_or(false),
                loop_at_end_of_playlist: settings.loop_at_end_of_playlist.unwrap_or(false),
                loop_single_files: settings.loop_single_files.unwrap_or(false),
                unpause_action_label: settings
                    .unpause_action
                    .clone()
                    .map(unpause_action_mode_legacy_name_compatible)
                    .unwrap_or("IfAlreadyReady")
                    .to_owned(),
                autoplay_min_users_label: settings
                    .autoplay_min_users
                    .as_ref()
                    .map(autoplay_threshold_override_legacy_value_compatible)
                    .unwrap_or_else(|| "app-default".to_owned()),
            },
            privacy: GuiPrivacySection {
                filename_privacy_mode_label: settings
                    .filename_privacy_mode
                    .map(privacy_mode_legacy_name_compatible)
                    .unwrap_or("SendRaw")
                    .to_owned(),
                filesize_privacy_mode_label: settings
                    .filesize_privacy_mode
                    .map(privacy_mode_legacy_name_compatible)
                    .unwrap_or("SendRaw")
                    .to_owned(),
                only_switch_to_trusted_domains: settings
                    .only_switch_to_trusted_domains
                    .unwrap_or(false),
                trusted_domains_text: optional_string_list_multiline_text(
                    settings.trusted_domains.as_deref(),
                ),
                trusted_domain_count: settings.trusted_domains.as_ref().map_or(0, Vec::len),
            },
            desync: GuiDesyncSection {
                rewind_on_desync: settings.rewind_on_desync.unwrap_or(false),
                fastforward_on_desync: settings.fastforward_on_desync.unwrap_or(false),
                slow_on_desync: settings.slow_on_desync.unwrap_or(false),
                dont_slow_down_with_me: settings.dont_slow_down_with_me.unwrap_or(false),
                rewind_threshold_seconds: settings.rewind_threshold_seconds,
                fastforward_threshold_seconds: settings.fastforward_threshold_seconds,
                slowdown_threshold_seconds: settings.slowdown_threshold_seconds,
            },
            media_search: GuiMediaSearchSection {
                media_directories_text: optional_string_list_multiline_text(
                    settings.media_search_directories.as_deref(),
                ),
                media_directory_count: settings
                    .media_search_directories
                    .as_ref()
                    .map_or(0, Vec::len),
                folder_search_first_file_timeout_seconds: settings
                    .folder_search_first_file_timeout_seconds,
                folder_search_timeout_seconds: settings.folder_search_timeout_seconds,
                folder_search_double_check_interval_seconds: settings
                    .folder_search_double_check_interval_seconds,
                folder_search_warning_threshold_seconds: settings
                    .folder_search_warning_threshold_seconds,
            },
            chat: GuiChatSection {
                chat_input_enabled: settings.chat_input_enabled.unwrap_or(false),
                chat_output_enabled: settings.chat_output_enabled.unwrap_or(false),
                chat_direct_input: settings.chat_direct_input.unwrap_or(false),
                chat_move_osd: settings.chat_move_osd.unwrap_or(false),
                chat_max_lines: settings.chat_max_lines,
                chat_input_position_label: settings
                    .chat_input_position
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Top")
                    .to_owned(),
                chat_input_font_family: settings.chat_input_font_family.clone(),
                chat_input_relative_font_size: settings.chat_input_relative_font_size,
                chat_input_font_weight: settings.chat_input_font_weight,
                chat_input_font_color: settings.chat_input_font_color.clone(),
                chat_output_mode_label: settings
                    .chat_output_mode
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Chatroom")
                    .to_owned(),
                chat_output_font_family: settings.chat_output_font_family.clone(),
                chat_output_relative_font_size: settings.chat_output_relative_font_size,
                chat_output_font_weight: settings.chat_output_font_weight,
                chat_top_margin: settings.chat_top_margin,
                chat_left_margin: settings.chat_left_margin,
                chat_bottom_margin: settings.chat_bottom_margin,
                chat_osd_margin: settings.chat_osd_margin,
            },
            osd: GuiOsdSection {
                show_osd: settings.show_osd.unwrap_or(false),
                show_duration_notification: settings.show_duration_notification.unwrap_or(false),
                show_same_room_osd: settings.show_same_room_osd.unwrap_or(false),
                show_osd_warnings: settings.show_osd_warnings.unwrap_or(false),
                show_slowdown_osd: settings.show_slowdown_osd.unwrap_or(false),
                show_noncontroller_osd: settings.show_noncontroller_osd.unwrap_or(false),
                show_different_room_osd: settings.show_different_room_osd.unwrap_or(false),
                show_contact_info: settings.show_contact_info.unwrap_or(false),
                notification_timeout_seconds: settings.notification_timeout_seconds,
                alert_timeout_seconds: settings.alert_timeout_seconds,
                chat_timeout_seconds: settings.chat_timeout_seconds,
            },
            system: GuiSystemSection {
                language_tag: settings
                    .language
                    .as_deref()
                    .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
                    .unwrap_or("en")
                    .to_owned(),
                check_for_updates_automatically: settings
                    .check_for_updates_automatically
                    .unwrap_or(false),
                autosave_joins_to_list: settings.autosave_joins_to_list.unwrap_or(false),
                force_gui_prompt: settings.force_gui_prompt.unwrap_or(false),
                compatibility_startup_entry_count: startup_entries.len(),
                ignored_startup_exception_count,
            },
        }
    }

    pub(super) fn dialog_sections(&self) -> Vec<GuiDialogSection> {
        vec![
            GuiDialogSection {
                title: "Connection",
                controls: vec![
                    GuiDialogControl {
                        label: "Host",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.host.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Port",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_port_text(self.connection.port),
                    },
                    GuiDialogControl {
                        label: "Username",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.username.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Room",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_room_text(self.connection.room.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Server Password",
                        kind: GuiDialogControlKind::PasswordInput,
                        value: if self.connection.server_password_set {
                            "(configured)".to_owned()
                        } else {
                            "(unset)".to_owned()
                        },
                    },
                    GuiDialogControl {
                        label: "Player Path",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.connection.player_path.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Player Arguments",
                        kind: GuiDialogControlKind::TextInput,
                        value: self.connection.player_arguments_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Public Servers",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.public_server_count.to_string(),
                    },
                    GuiDialogControl {
                        label: "Room History",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.connection.room_history_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Room History Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.room_history_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Readiness",
                controls: vec![
                    GuiDialogControl {
                        label: "Ready At Start",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.ready_at_start).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Autoplay",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Require Same Filenames",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.autoplay_require_same_filenames)
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Shared Playlists",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.shared_playlist_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Pause On Leave",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.pause_on_leave).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Loop At End Of Playlist",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_at_end_of_playlist).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Loop Single Files",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.readiness.loop_single_files).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Unpause Action",
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.unpause_action_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Autoplay Min Users",
                        kind: GuiDialogControlKind::Select,
                        value: self.readiness.autoplay_min_users_label.clone(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Privacy",
                controls: vec![
                    GuiDialogControl {
                        label: "Filename Privacy",
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filename_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Filesize Privacy",
                        kind: GuiDialogControlKind::Select,
                        value: self.privacy.filesize_privacy_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domains Only",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.privacy.only_switch_to_trusted_domains).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domains",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.privacy.trusted_domains_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Trusted Domain Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.privacy.trusted_domain_count.to_string(),
                    },
                ],
            },
            GuiDialogSection {
                title: "Desync",
                controls: vec![
                    GuiDialogControl {
                        label: "Rewind On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.rewind_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Fastforward On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.fastforward_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Slow On Desync",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.slow_on_desync).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Dont Slow Down With Me",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.desync.dont_slow_down_with_me).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Rewind Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.rewind_threshold_seconds),
                    },
                    GuiDialogControl {
                        label: "Fastforward Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.fastforward_threshold_seconds),
                    },
                    GuiDialogControl {
                        label: "Slowdown Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.desync.slowdown_threshold_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "Media Search",
                controls: vec![
                    GuiDialogControl {
                        label: "Directories",
                        kind: GuiDialogControlKind::TextArea,
                        value: self.media_search.media_directories_text.clone(),
                    },
                    GuiDialogControl {
                        label: "Directory Count",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.media_search.media_directory_count.to_string(),
                    },
                    GuiDialogControl {
                        label: "First File Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search.folder_search_first_file_timeout_seconds,
                        ),
                    },
                    GuiDialogControl {
                        label: "Search Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(self.media_search.folder_search_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Double Check Interval",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search
                                .folder_search_double_check_interval_seconds,
                        ),
                    },
                    GuiDialogControl {
                        label: "Warning Threshold",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_f64_text(
                            self.media_search.folder_search_warning_threshold_seconds,
                        ),
                    },
                ],
            },
            GuiDialogSection {
                title: "Chat",
                controls: vec![
                    GuiDialogControl {
                        label: "Chat Input",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_input_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Chat Output",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_output_enabled).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Direct Input",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_direct_input).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Move OSD",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.chat.chat_move_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Input Position",
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_input_position_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Output Mode",
                        kind: GuiDialogControlKind::Select,
                        value: self.chat.chat_output_mode_label.clone(),
                    },
                    GuiDialogControl {
                        label: "Max Lines",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_max_lines),
                    },
                    GuiDialogControl {
                        label: "Input Font",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Input Font Size",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_relative_font_size),
                    },
                    GuiDialogControl {
                        label: "Input Font Weight",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_input_font_weight),
                    },
                    GuiDialogControl {
                        label: "Input Color",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_input_font_color.as_deref()).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Output Font",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_output_font_family.as_deref())
                            .to_owned(),
                    },
                    GuiDialogControl {
                        label: "Output Font Size",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_relative_font_size),
                    },
                    GuiDialogControl {
                        label: "Output Font Weight",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_output_font_weight),
                    },
                    GuiDialogControl {
                        label: "Top Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_top_margin),
                    },
                    GuiDialogControl {
                        label: "Left Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_left_margin),
                    },
                    GuiDialogControl {
                        label: "Bottom Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_bottom_margin),
                    },
                    GuiDialogControl {
                        label: "OSD Margin",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.chat.chat_osd_margin),
                    },
                ],
            },
            GuiDialogSection {
                title: "OSD",
                controls: vec![
                    GuiDialogControl {
                        label: "Show OSD",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Duration",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_duration_notification).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Same Room",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_same_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Warnings",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_osd_warnings).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Slowdown",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_slowdown_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Noncontroller",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_noncontroller_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Different Room",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_different_room_osd).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Show Contact Info",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.osd.show_contact_info).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Notification Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.notification_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Alert Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.alert_timeout_seconds),
                    },
                    GuiDialogControl {
                        label: "Chat Timeout",
                        kind: GuiDialogControlKind::NumericInput,
                        value: optional_i64_text(self.osd.chat_timeout_seconds),
                    },
                ],
            },
            GuiDialogSection {
                title: "System",
                controls: vec![
                    GuiDialogControl {
                        label: "Language",
                        kind: GuiDialogControlKind::Select,
                        value: self.system.language_tag.clone(),
                    },
                    GuiDialogControl {
                        label: "Auto Update",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.check_for_updates_automatically).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Autosave Joins To List",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.autosave_joins_to_list).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Force GUI Prompt",
                        kind: GuiDialogControlKind::Checkbox,
                        value: bool_label(self.system.force_gui_prompt).to_owned(),
                    },
                    GuiDialogControl {
                        label: "Supported Languages",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.to_owned(),
                    },
                ],
            },
        ]
    }
}
