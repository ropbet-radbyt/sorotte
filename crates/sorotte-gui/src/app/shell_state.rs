use sorotte_client_app::app_boundary::{
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
    storage::SorotteClientStoragePaths,
};
use sorotte_media_match::{MediaMatchAutoplayPolicy, MediaMatchSettings};
use sorotte_plex::PlexServerConnectionKind;

use super::GuiLaunchMode;
use super::remote_services;
use super::support::{
    bool_label, legacy_chat_input_enabled, legacy_chat_output_enabled, optional_f64_text,
    optional_i64_text, optional_port_text, optional_room_text, optional_string_list_multiline_text,
    optional_text, player_arguments_text_for_path,
};
use super::ui_state::GuiUpdateCheckState;
use super::widget_tree::GuiWidgetKind;

#[cfg(test)]
mod tests;

mod actions;
mod browser_support;
mod configuration_dialog;
mod configuration_dialog_projection;
mod main_window;

pub(super) use self::actions::GuiShellAction;
pub(super) use self::browser_support::{
    browser_domain_from_url, browser_format_duration_label, browser_format_size_label,
    browser_is_url, browser_stream_target_kind, browser_uri_is_trusted,
    load_playlist_entries_from_path, playlist_entries_from_multiline_text,
    playlist_entries_multiline_text, save_playlist_entries_to_path,
    shuffle_playlist_entries_in_place,
};
pub(super) use self::configuration_dialog::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, GuiChatSection,
    GuiConnectionSettingsSection, GuiDesyncSection, GuiDialogControl, GuiDialogControlKind,
    GuiDialogSection, GuiMediaSearchSection, GuiOsdSection, GuiPrivacySection, GuiReadinessSection,
    GuiSystemSection,
};
#[cfg(any(test, feature = "gui-semantic-smoke"))]
pub(super) use self::main_window::MainWindowRuntimeChatSnapshot;
pub(super) use self::main_window::{
    MainWindowChatRow, MainWindowPlaybackControls, MainWindowPlaylistRow, MainWindowRoomRow,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    MainWindowShellState, MainWindowUserRow,
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

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GuiStreamHelperRemediationState {
    pub(super) active: bool,
    pub(super) label: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GuiStreamHelperRemediationRuntimeSnapshot {
    pub(super) active: bool,
    pub(super) label: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
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
    #[cfg(test)]
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
pub(super) enum GuiStreamTargetKind {
    LocalPath,
    DirectMediaUrl,
    ExtractorPageUrl,
    UntrustedUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiStreamHelperHealth {
    Healthy,
    MissingDownloader,
    MissingJsRuntime,
    Stale,
    Broken,
    UnsupportedPlatform,
    ExternalPlayerUnmanaged,
}

impl GuiStreamHelperHealth {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::MissingDownloader => "missing-downloader",
            Self::MissingJsRuntime => "missing-js-runtime",
            Self::Stale => "stale",
            Self::Broken => "broken",
            Self::UnsupportedPlatform => "unsupported-platform",
            Self::ExternalPlayerUnmanaged => "external-player-unmanaged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiStreamHelperState {
    pub(super) health: GuiStreamHelperHealth,
    pub(super) message: Option<String>,
    pub(super) target: Option<String>,
    pub(super) install_supported: bool,
    pub(super) integration_supported: bool,
    pub(super) retry_available: bool,
    pub(super) install_location: Option<String>,
    pub(super) downloader_status: Option<String>,
    pub(super) js_runtime_status: Option<String>,
    pub(super) open_install_location_available: bool,
}

impl Default for GuiStreamHelperState {
    fn default() -> Self {
        Self {
            health: GuiStreamHelperHealth::Healthy,
            message: None,
            target: None,
            install_supported: false,
            integration_supported: false,
            retry_available: false,
            install_location: None,
            downloader_status: None,
            js_runtime_status: None,
            open_install_location_available: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiStreamHelperRuntimeSnapshot {
    pub(super) health: GuiStreamHelperHealth,
    pub(super) message: Option<String>,
    pub(super) target: Option<String>,
    pub(super) install_supported: bool,
    pub(super) integration_supported: bool,
    pub(super) retry_available: bool,
    pub(super) install_location: Option<String>,
    pub(super) downloader_status: Option<String>,
    pub(super) js_runtime_status: Option<String>,
    pub(super) open_install_location_available: bool,
}

impl Default for GuiStreamHelperRuntimeSnapshot {
    fn default() -> Self {
        Self {
            health: GuiStreamHelperHealth::Healthy,
            message: None,
            target: None,
            install_supported: false,
            integration_supported: false,
            retry_available: false,
            install_location: None,
            downloader_status: None,
            js_runtime_status: None,
            open_install_location_available: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiMediaMatchToolHealth {
    Healthy,
    MissingFfmpeg,
    MissingFfprobe,
    Broken,
}

impl GuiMediaMatchToolHealth {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::MissingFfmpeg => "missing-ffmpeg",
            Self::MissingFfprobe => "missing-ffprobe",
            Self::Broken => "broken",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiMediaMatchState {
    pub(super) settings: MediaMatchSettings,
    pub(super) health: GuiMediaMatchToolHealth,
    pub(super) message: Option<String>,
    pub(super) install_supported: bool,
    pub(super) integration_supported: bool,
    pub(super) install_location: Option<String>,
    pub(super) ffmpeg_status: Option<String>,
    pub(super) ffprobe_status: Option<String>,
    pub(super) fpcalc_status: Option<String>,
    pub(super) cache_status: Option<String>,
    pub(super) current_decision: Option<String>,
    pub(super) nearest_match: Option<String>,
    pub(super) last_evidence: Option<String>,
    pub(super) remote_status: Option<String>,
    pub(super) background_status: Option<String>,
    pub(super) open_install_location_available: bool,
}

impl GuiMediaMatchState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        Self {
            settings: media_match_settings_from_stored_settings(settings),
            ..Self::default()
        }
    }
}

impl Default for GuiMediaMatchState {
    fn default() -> Self {
        Self {
            settings: MediaMatchSettings::default(),
            health: GuiMediaMatchToolHealth::MissingFfmpeg,
            message: Some("Media Matching needs ffmpeg for frame extraction.".to_owned()),
            install_supported: cfg!(windows),
            integration_supported: true,
            install_location: None,
            ffmpeg_status: None,
            ffprobe_status: None,
            fpcalc_status: None,
            cache_status: Some("empty".to_owned()),
            current_decision: None,
            nearest_match: None,
            last_evidence: None,
            remote_status: Some("unavailable".to_owned()),
            background_status: Some("idle".to_owned()),
            open_install_location_available: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiMediaMatchRuntimeSnapshot {
    pub(super) settings: MediaMatchSettings,
    pub(super) health: GuiMediaMatchToolHealth,
    pub(super) message: Option<String>,
    pub(super) install_supported: bool,
    pub(super) integration_supported: bool,
    pub(super) install_location: Option<String>,
    pub(super) ffmpeg_status: Option<String>,
    pub(super) ffprobe_status: Option<String>,
    pub(super) fpcalc_status: Option<String>,
    pub(super) cache_status: Option<String>,
    pub(super) current_decision: Option<String>,
    pub(super) nearest_match: Option<String>,
    pub(super) last_evidence: Option<String>,
    pub(super) remote_status: Option<String>,
    pub(super) background_status: Option<String>,
    pub(super) open_install_location_available: bool,
}

impl From<&GuiMediaMatchState> for GuiMediaMatchRuntimeSnapshot {
    fn from(value: &GuiMediaMatchState) -> Self {
        Self {
            settings: value.settings.clone(),
            health: value.health,
            message: value.message.clone(),
            install_supported: value.install_supported,
            integration_supported: value.integration_supported,
            install_location: value.install_location.clone(),
            ffmpeg_status: value.ffmpeg_status.clone(),
            ffprobe_status: value.ffprobe_status.clone(),
            fpcalc_status: value.fpcalc_status.clone(),
            cache_status: value.cache_status.clone(),
            current_decision: value.current_decision.clone(),
            nearest_match: value.nearest_match.clone(),
            last_evidence: value.last_evidence.clone(),
            remote_status: value.remote_status.clone(),
            background_status: value.background_status.clone(),
            open_install_location_available: value.open_install_location_available,
        }
    }
}

impl Default for GuiMediaMatchRuntimeSnapshot {
    fn default() -> Self {
        Self::from(&GuiMediaMatchState::default())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GuiMediaMatchRemediationState {
    pub(super) active: bool,
    pub(super) label: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GuiMediaMatchRemediationRuntimeSnapshot {
    pub(super) active: bool,
    pub(super) label: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlexServerRow {
    pub(super) name: String,
    pub(super) machine_identifier: String,
    pub(super) uri: String,
    pub(super) reachability: GuiPlexServerReachability,
    pub(super) connection_kind: PlexServerConnectionKind,
    pub(super) has_local_connection: bool,
    pub(super) owned: bool,
    pub(super) selected: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum GuiPlexServerReachability {
    #[default]
    Unknown,
    Checking,
    Reachable,
    Unreachable,
}

impl GuiPlexServerReachability {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Checking => "checking",
            Self::Reachable => "reachable",
            Self::Unreachable => "offline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlexState {
    pub(super) enabled: bool,
    pub(super) authenticated: bool,
    pub(super) authenticating: bool,
    pub(super) auth_code: Option<String>,
    pub(super) auth_url: Option<String>,
    pub(super) selected_server_id: Option<String>,
    pub(super) selected_server_url: Option<String>,
    pub(super) servers: Vec<GuiPlexServerRow>,
    pub(super) status: String,
    pub(super) current_item: Option<String>,
    pub(super) last_report: Option<String>,
    pub(super) last_error: Option<String>,
}

impl GuiPlexState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let authenticated = settings
            .plex_user_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
        let selected_server_id = settings.plex_selected_server_id.clone();
        let selected_server_url = settings.plex_selected_server_url.clone();
        Self {
            enabled: settings.plex_sync_enabled.unwrap_or(false),
            authenticated,
            authenticating: false,
            auth_code: None,
            auth_url: None,
            selected_server_id,
            selected_server_url,
            servers: Vec::new(),
            status: if authenticated {
                "ready".to_owned()
            } else {
                "disconnected".to_owned()
            },
            current_item: None,
            last_report: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiPlexRuntimeSnapshot {
    pub(super) enabled: bool,
    pub(super) authenticated: bool,
    pub(super) authenticating: bool,
    pub(super) auth_code: Option<String>,
    pub(super) auth_url: Option<String>,
    pub(super) selected_server_id: Option<String>,
    pub(super) selected_server_url: Option<String>,
    pub(super) servers: Vec<GuiPlexServerRow>,
    pub(super) status: String,
    pub(super) current_item: Option<String>,
    pub(super) last_report: Option<String>,
    pub(super) last_error: Option<String>,
}

impl From<&GuiPlexState> for GuiPlexRuntimeSnapshot {
    fn from(value: &GuiPlexState) -> Self {
        Self {
            enabled: value.enabled,
            authenticated: value.authenticated,
            authenticating: value.authenticating,
            auth_code: value.auth_code.clone(),
            auth_url: value.auth_url.clone(),
            selected_server_id: value.selected_server_id.clone(),
            selected_server_url: value.selected_server_url.clone(),
            servers: value.servers.clone(),
            status: value.status.clone(),
            current_item: value.current_item.clone(),
            last_report: value.last_report.clone(),
            last_error: value.last_error.clone(),
        }
    }
}

impl Default for GuiPlexRuntimeSnapshot {
    fn default() -> Self {
        Self::from(&GuiPlexState::from_stored_settings(
            &StoredClientSettingsMvp::default(),
        ))
    }
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
pub(super) struct SorotteGuiRuntimeSnapshot {
    pub(super) active_view: GuiShellView,
    pub(super) open_modal: Option<GuiShellModal>,
    pub(super) main_window: MainWindowRuntimeSnapshot,
    pub(super) public_servers: PublicServerBrowserShellState,
    pub(super) media_search: MediaSearchWorkflowShellState,
    pub(super) tls_prompt_expected: bool,
    pub(super) update_notice_expected: bool,
    pub(super) about_dialog_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum GuiPluginSelection {
    #[default]
    StreamSupport,
    MediaMatching,
    Plex,
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
pub(super) struct SorotteGuiShellAppState {
    pub(super) active_view: GuiShellView,
    pub(super) selected_configuration_tab: GuiConfigurationTab,
    pub(super) selected_plugin: GuiPluginSelection,
    pub(super) open_modal: Option<GuiShellModal>,
    pub(super) selection: GuiSelectionState,
    pub(super) main_window_playlist_selection_is_local: bool,
    pub(super) runtime_menu_action_overrides: Vec<MenuActionRuntimeOverride>,
    pub(super) runtime_command_availability_override: GuiCommandAvailabilityRuntimeOverride,
    pub(super) config_storage: GuiConfigStorageRuntimeSnapshot,
    pub(super) commands: GuiCommandAvailabilityState,
    pub(super) pending_operation: Option<GuiPendingOperationState>,
    pub(super) pending_config_storage_target: Option<GuiConfigStorageChangeTarget>,
    pub(super) pending_local_ready_target: Option<bool>,
    pub(super) pending_saved_server_connect_saves_configuration: bool,
    pub(super) outgoing_chat_message: Option<String>,
    pub(super) main_window_room_change_expanded: bool,
    pub(super) new_main_window_user_draft: String,
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
    pub(super) stream_helper: GuiStreamHelperState,
    pub(super) stream_helper_remediation: GuiStreamHelperRemediationState,
    pub(super) media_match: GuiMediaMatchState,
    pub(super) media_match_remediation: GuiMediaMatchRemediationState,
    pub(super) plex: GuiPlexState,
    pub(super) saved_configuration: StoredClientSettingsMvp,
    pub(super) configuration: FirstRunConfigurationDialogDraft,
    pub(super) main_window: MainWindowShellState,
    pub(super) menus: MenuDialogShellState,
    pub(super) public_servers: PublicServerBrowserShellState,
    pub(super) media_search: MediaSearchWorkflowShellState,
}

pub(super) fn media_match_settings_from_stored_settings(
    settings: &StoredClientSettingsMvp,
) -> MediaMatchSettings {
    let mut media_match_settings = MediaMatchSettings::default();
    if let Some(enabled) = settings.media_match_fingerprinting_enabled {
        media_match_settings.fingerprinting_enabled = enabled;
    }
    if let Some(enabled) = settings.media_match_background_warmup_enabled {
        media_match_settings.background_warmup_enabled = enabled;
    }
    if let Some(enabled) = settings.media_match_wire_sharing_enabled {
        media_match_settings.wire_sharing_enabled = enabled;
    }
    if let Some(enabled) = settings.media_match_runtime_tolerance_enabled {
        media_match_settings.runtime_tolerance_enabled = enabled;
    }
    media_match_settings.autoplay_policy = settings
        .media_match_autoplay_policy
        .as_deref()
        .and_then(media_match_autoplay_policy_from_label)
        .unwrap_or_default();
    media_match_settings
}

pub(super) fn apply_media_match_settings_to_stored_settings(
    settings: &mut StoredClientSettingsMvp,
    media_match_settings: &MediaMatchSettings,
) {
    settings.media_match_fingerprinting_enabled = Some(media_match_settings.fingerprinting_enabled);
    settings.media_match_background_warmup_enabled =
        Some(media_match_settings.background_warmup_enabled);
    settings.media_match_wire_sharing_enabled = Some(media_match_settings.wire_sharing_enabled);
    settings.media_match_runtime_tolerance_enabled =
        Some(media_match_settings.runtime_tolerance_enabled);
    settings.media_match_autoplay_policy =
        Some(media_match_autoplay_policy_label(media_match_settings.autoplay_policy).to_owned());
}

pub(super) fn media_match_autoplay_policy_label(policy: MediaMatchAutoplayPolicy) -> &'static str {
    match policy {
        MediaMatchAutoplayPolicy::DiagnosticsOnly => "DiagnosticsOnly",
        MediaMatchAutoplayPolicy::AllowStrongSameMedia => "AllowStrongSameMedia",
    }
}

pub(super) fn media_match_autoplay_policy_from_label(
    value: &str,
) -> Option<MediaMatchAutoplayPolicy> {
    match value.trim().to_ascii_lowercase().as_str() {
        "diagnosticsonly" | "diagnostics-only" | "diagnostics_only" => {
            Some(MediaMatchAutoplayPolicy::DiagnosticsOnly)
        }
        "allowstrongsamemedia" | "allow-strong-same-media" | "allow_strong_same_media" => {
            Some(MediaMatchAutoplayPolicy::AllowStrongSameMedia)
        }
        _ => None,
    }
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
    pub(super) chat_unavailable_reason: Option<String>,
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
    pub(super) chat_unavailable_reason: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiCommandRuntimeSnapshot {
    pub(super) command_availability: GuiCommandAvailabilityState,
    pub(super) pending_operation: Option<GuiPendingOperationKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct GuiConfigStorageRuntimeSnapshot {
    pub(super) config_path: Option<String>,
    pub(super) storage_root: Option<String>,
    pub(super) default_storage_root: Option<String>,
    pub(super) source_label: String,
    pub(super) external_override_active: bool,
}

impl GuiConfigStorageRuntimeSnapshot {
    pub(super) fn from_storage_paths(paths: &SorotteClientStoragePaths) -> Self {
        Self {
            config_path: Some(paths.config_path.to_string_lossy().into_owned()),
            storage_root: Some(paths.storage_root.to_string_lossy().into_owned()),
            default_storage_root: Some(paths.default_storage_root.to_string_lossy().into_owned()),
            source_label: paths.source.label().to_owned(),
            external_override_active: paths.source.is_external_override(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiConfigStorageChangeTarget {
    CustomRoot(String),
    DefaultRoot,
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
    pub(super) fn from_shell_state(state: &SorotteGuiShellAppState) -> Self {
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
    ChangeConfigStorageRoot,
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
            Self::ChangeConfigStorageRoot => "change-config-storage-root",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum GuiShellView {
    #[default]
    Setup,
    Room,
    Plugins,
}

impl GuiShellView {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Room => "room",
            Self::Plugins => "plugins",
        }
    }

    pub(super) fn from_label(label: &str) -> Option<Self> {
        match label {
            "setup" | "configuration" | "menus-and-dialogs" | "public-servers" | "media-search" => {
                Some(Self::Setup)
            }
            "room" | "main-window" => Some(Self::Room),
            "plugins" | "stream-support" => Some(Self::Plugins),
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
    StreamSupport,
}

impl GuiShellModal {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::UpdateNotice => "update-notice",
            Self::TlsCertificatePrompt => "tls-certificate-prompt",
            Self::PlayerSetup => "player-setup",
            Self::StreamSupport => "stream-support",
        }
    }
}
