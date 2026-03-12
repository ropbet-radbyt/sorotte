#![allow(dead_code)]

#[path = "app_configuration_draft.rs"]
mod configuration_draft;
#[path = "app_connection_workflows.rs"]
mod connection_workflows;
#[path = "app_feedback_workflows.rs"]
mod feedback_workflows;
#[path = "app_media_workflows.rs"]
mod media_workflows;
#[path = "mpv_launch.rs"]
mod mpv_launch;
#[path = "app_native_host.rs"]
mod native_host;
#[path = "app_reducer.rs"]
mod reducer;
#[path = "remote_services.rs"]
mod remote_services;
#[path = "app_runtime_queue.rs"]
mod runtime_queue;
#[path = "app_runtime_updates.rs"]
mod runtime_updates;
#[path = "app_shell_projection.rs"]
mod shell_projection;
#[path = "app_shell_workflows.rs"]
mod shell_workflows;
#[path = "app_startup.rs"]
mod startup;
#[path = "app_startup_support.rs"]
mod startup_support;
#[path = "app_state_integrity.rs"]
mod state_integrity;
#[path = "app_support.rs"]
mod support;
#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
#[path = "app_ui_state.rs"]
mod ui_state;
#[path = "app_widget_projection.rs"]
mod widget_projection;
#[path = "app_widget_tree.rs"]
mod widget_tree;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    io::{self, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use eframe::egui;
use rfd::FileDialog;
use serde_json::Value;
use sha2::{Digest, Sha256};
use syncplay_client_app::app_boundary::{
    commands::{
        LocalInputCommand, LocalOffsetCommand, PlannedLocalRuntimeAction,
        controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
        localized_current_offset_message_legacy_compatible, parse_local_input_command,
        plan_local_offset_runtime_dispatch_legacy_compatible,
    },
    compatibility::{
        LegacyConfigurationGetterCompatibilityStatus,
        legacy_configuration_getter_startup_compat_entries,
    },
    language::{
        SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
        normalized_legacy_runtime_language_tag_legacy_compatible,
    },
    persistence::{
        clear_syncplay_ini_stored_client_settings_mvp_at_path,
        format_serialized_public_servers_list_legacy_compatible,
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        parse_serialized_public_servers_list_legacy_compatible,
        parse_serialized_string_list_legacy_compatible,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    },
    state::{
        AutoplayThresholdOverride, StoredClientSettingsMvp, StoredClientSettingsRuntimeSnapshot,
        autoplay_threshold_override_legacy_value_compatible,
        parse_autoplay_min_users_override_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible,
        parse_unpause_action_mode_legacy_compatible, privacy_mode_legacy_name_compatible,
        stored_client_settings_runtime_snapshot_legacy_compatible,
        unpause_action_mode_legacy_name_compatible,
    },
};
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, AutoplayCountdownNotification, ChatNotification, ClientRuntime,
    ClientSession, ControlledRoomCreationNotification, ControllerAuthTransitionNotification,
    PrivacyMode, QueuedRuntimeControl, ReconnectTransitionNotification, UserChangeNotification,
};
use syncplay_player_api::{LocalFileUpdate, PlayerAdapter, PlayerPlaybackTelemetryUpdate};
use syncplay_player_mpv::{LegacySyncplayOsdKind, LegacySyncplayUiSettings, MpvAdapter};

use self::mpv_launch::{
    ManagedMpvLaunchConfig, ManagedMpvProcessGuard, ManagedMpvSettingsDecision,
    apply_legacy_syncplay_ui_settings_to_mpv_adapter, managed_mpv_settings_decision_from_settings,
};
use self::native_host::GuiEframeNativeHost;
#[cfg(test)]
use self::native_host::{GuiNativeApp, GuiTextPreviewHost};
use self::runtime_queue::{
    GuiQueuedRuntimeBridge, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwnerPump,
};
use self::startup::{
    explicit_mpv_ipc_path_from_lookup, gui_startup_actions_from_lookup,
    gui_startup_host_and_settings, load_gui_ui_state_from_lookup,
    resolve_syncplay_gui_config_path_legacy_compatible,
    run_gui_host_with_startup_actions_and_gui_state, syncplay_gui_qsettings_root_from_env,
};
#[cfg(test)]
use self::startup::{
    gui_startup_actions_from_lookup_and_config_path_source,
    gui_startup_remote_actions_with_fetchers, gui_startup_settings_from_lookup,
    gui_startup_settings_from_lookup_with,
    resolve_syncplay_gui_config_path_source_legacy_compatible_with, run_gui_host,
    run_gui_host_with_startup_actions, shell_widget_preview, startup_notice, startup_preview,
};
#[cfg(test)]
use self::startup_support::{GuiClientCoreChatLoopbackBootstrap, GuiClientCoreChatTcpBootstrap};
use self::startup_support::{
    GuiStartupConfigPathSource, GuiStartupPlayerIpcSource, GuiStartupPublicServerSource,
    env_flag_enabled_lookup, env_trimmed, gui_client_core_chat_loopback_bootstrap_from_lookup,
    gui_client_core_chat_tcp_bootstrap_from_lookup,
};
use self::support::{
    autoplay_threshold_from_settings, bool_label, format_offset_command, normalized_editable_text,
    optional_f64_text, optional_i64_text, optional_index_text, optional_port_text,
    optional_seconds_text, optional_string_list_text, optional_text, parse_trusted_domains_text,
    system_time_seconds,
};
#[cfg(test)]
use self::ui_state::legacy_gui_qsettings_store_path;
use self::ui_state::{
    GuiPersistedUiState, GuiUpdateCheckState, clear_legacy_gui_qsettings_files_at_root,
    load_gui_ui_state_from_root, persist_gui_ui_state_at_root,
};
#[cfg(test)]
use self::widget_tree::GuiWidgetTextPreviewRenderer;
use self::widget_tree::{GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiLaunchMode {
    FirstRun,
    ExistingConfig,
}

impl GuiLaunchMode {
    fn label(self) -> &'static str {
        match self {
            Self::FirstRun => "first-run",
            Self::ExistingConfig => "existing-config",
        }
    }
}

const LEGACY_GUI_QSETTINGS_STORE_NAMES: [&str; 5] = [
    "PlayerList",
    "MediaBrowseDialog",
    "MainWindow",
    "Interface",
    "MoreSettings",
];
const DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiConnectionSettingsSection {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    room: Option<String>,
    server_password_set: bool,
    player_path: Option<String>,
    public_server_count: usize,
    room_history_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiReadinessSection {
    ready_at_start: bool,
    autoplay_enabled: bool,
    autoplay_require_same_filenames: bool,
    shared_playlist_enabled: bool,
    pause_on_leave: bool,
    unpause_action_label: String,
    autoplay_min_users_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiPrivacySection {
    filename_privacy_mode_label: String,
    filesize_privacy_mode_label: String,
    only_switch_to_trusted_domains: bool,
    trusted_domains_label: String,
    trusted_domain_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiDesyncSection {
    rewind_on_desync: bool,
    fastforward_on_desync: bool,
    slow_on_desync: bool,
    dont_slow_down_with_me: bool,
    rewind_threshold_seconds: Option<f64>,
    fastforward_threshold_seconds: Option<f64>,
    slowdown_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiMediaSearchSection {
    media_directory_count: usize,
    folder_search_first_file_timeout_seconds: Option<f64>,
    folder_search_timeout_seconds: Option<f64>,
    folder_search_double_check_interval_seconds: Option<f64>,
    folder_search_warning_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiChatSection {
    chat_input_enabled: bool,
    chat_output_enabled: bool,
    chat_direct_input: bool,
    chat_move_osd: bool,
    chat_max_lines: Option<i64>,
    chat_input_font_family: Option<String>,
    chat_output_font_family: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiOsdSection {
    show_osd: bool,
    show_duration_notification: bool,
    show_same_room_osd: bool,
    show_osd_warnings: bool,
    show_noncontroller_osd: bool,
    show_different_room_osd: bool,
    show_contact_info: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiSystemSection {
    language_tag: String,
    check_for_updates_automatically: bool,
    compatibility_startup_entry_count: usize,
    ignored_startup_exception_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiDialogControlKind {
    TextInput,
    PasswordInput,
    Checkbox,
    Select,
    NumericInput,
    ReadOnly,
}

impl GuiDialogControlKind {
    fn label(self) -> &'static str {
        match self {
            Self::TextInput => "text",
            Self::PasswordInput => "password",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::NumericInput => "numeric",
            Self::ReadOnly => "readonly",
        }
    }

    fn is_editable(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiDialogControl {
    label: &'static str,
    kind: GuiDialogControlKind,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiDialogSection {
    title: &'static str,
    controls: Vec<GuiDialogControl>,
}

impl GuiDialogSection {
    fn control_mut(&mut self, label: &str) -> Option<&mut GuiDialogControl> {
        self.controls
            .iter_mut()
            .find(|control| control.label == label)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FirstRunConfigurationDialogState {
    launch_mode: GuiLaunchMode,
    connection: GuiConnectionSettingsSection,
    readiness: GuiReadinessSection,
    privacy: GuiPrivacySection,
    desync: GuiDesyncSection,
    media_search: GuiMediaSearchSection,
    chat: GuiChatSection,
    osd: GuiOsdSection,
    system: GuiSystemSection,
}

#[derive(Debug, Clone, PartialEq)]
struct FirstRunConfigurationDialogDraft {
    launch_mode: GuiLaunchMode,
    compatibility_startup_entry_count: usize,
    ignored_startup_exception_count: usize,
    sections: Vec<GuiDialogSection>,
    settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainWindowRoomRow {
    room_name: String,
    is_controlled: bool,
    has_named_users: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainWindowUserRow {
    username: String,
    room_name: String,
    is_self: bool,
    is_ready: bool,
    is_controller: bool,
    has_file: bool,
    file_name: Option<String>,
    file_name_label: String,
    file_size_label: String,
    file_duration_label: String,
    file_is_url: bool,
    file_is_trusted: bool,
    filename_differs: bool,
    filesize_differs: bool,
    fileduration_differs: bool,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainWindowPlaylistRow {
    label: String,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainWindowChatRow {
    sender: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainWindowPlaybackControls {
    can_toggle_pause: bool,
    can_seek: bool,
    can_undo_seek: bool,
    can_set_offset: bool,
    can_toggle_autoplay: bool,
    can_adjust_autoplay_threshold: bool,
    can_set_ready: bool,
    can_set_others_ready: bool,
    can_manage_playlist: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct MainWindowShellState {
    room_name: String,
    shared_playlist_enabled: bool,
    controlled_room_active: bool,
    hide_empty_rooms: bool,
    rooms: Vec<MainWindowRoomRow>,
    users: Vec<MainWindowUserRow>,
    playlist: Vec<MainWindowPlaylistRow>,
    chat: Vec<MainWindowChatRow>,
    playback: MainWindowPlaybackControls,
    playback_paused: bool,
    autoplay_active: bool,
    autoplay_threshold: usize,
    autoplay_countdown_seconds: Option<u32>,
    user_offset_seconds: f64,
    show_playback_buttons: bool,
    show_autoplay_controls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MainWindowRuntimeUserSnapshot {
    username: String,
    room_name: String,
    is_self: bool,
    is_ready: bool,
    is_controller: bool,
    has_file: bool,
    file_name: Option<String>,
    file_size_label: String,
    file_duration_label: String,
    file_is_url: bool,
    file_is_trusted: bool,
    filename_differs: bool,
    filesize_differs: bool,
    fileduration_differs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MainWindowRuntimeRoomSnapshot {
    room_name: String,
    is_controlled: bool,
    has_named_users: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MainWindowRuntimeChatSnapshot {
    sender: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq)]
struct MainWindowRuntimeSnapshot {
    room_name: String,
    shared_playlist_enabled: bool,
    controlled_room_active: bool,
    hide_empty_rooms: bool,
    rooms: Vec<MainWindowRuntimeRoomSnapshot>,
    users: Vec<MainWindowRuntimeUserSnapshot>,
    playlist: Vec<String>,
    chat: Vec<MainWindowRuntimeChatSnapshot>,
    can_toggle_pause: bool,
    can_seek: bool,
    can_undo_seek: bool,
    can_set_offset: bool,
    can_toggle_autoplay: bool,
    can_adjust_autoplay_threshold: bool,
    can_set_ready: bool,
    can_set_others_ready: bool,
    can_manage_playlist: bool,
    playback_paused: bool,
    autoplay_active: bool,
    autoplay_threshold: usize,
    autoplay_countdown_seconds: Option<u32>,
    user_offset_seconds: f64,
    show_playback_buttons: bool,
    show_autoplay_controls: bool,
}

impl Default for MainWindowRuntimeSnapshot {
    fn default() -> Self {
        Self {
            room_name: String::new(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            users: Vec::new(),
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_undo_seek: false,
            can_set_offset: false,
            can_toggle_autoplay: true,
            can_adjust_autoplay_threshold: true,
            can_set_ready: false,
            can_set_others_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            autoplay_threshold: DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD,
            autoplay_countdown_seconds: None,
            user_offset_seconds: 0.0,
            show_playback_buttons: true,
            show_autoplay_controls: true,
        }
    }
}

impl MainWindowRuntimeSnapshot {
    fn from_shell_state(state: &MainWindowShellState) -> Self {
        Self {
            room_name: state.room_name.clone(),
            shared_playlist_enabled: state.shared_playlist_enabled,
            controlled_room_active: state.controlled_room_active,
            hide_empty_rooms: state.hide_empty_rooms,
            rooms: state
                .rooms
                .iter()
                .map(|room| MainWindowRuntimeRoomSnapshot {
                    room_name: room.room_name.clone(),
                    is_controlled: room.is_controlled,
                    has_named_users: room.has_named_users,
                })
                .collect(),
            users: state
                .users
                .iter()
                .map(|user| MainWindowRuntimeUserSnapshot {
                    username: user.username.clone(),
                    room_name: user.room_name.clone(),
                    is_self: user.is_self,
                    is_ready: user.is_ready,
                    is_controller: user.is_controller,
                    has_file: user.has_file,
                    file_name: user.file_name.clone(),
                    file_size_label: user.file_size_label.clone(),
                    file_duration_label: user.file_duration_label.clone(),
                    file_is_url: user.file_is_url,
                    file_is_trusted: user.file_is_trusted,
                    filename_differs: user.filename_differs,
                    filesize_differs: user.filesize_differs,
                    fileduration_differs: user.fileduration_differs,
                })
                .collect(),
            playlist: state.playlist.iter().map(|row| row.label.clone()).collect(),
            chat: state
                .chat
                .iter()
                .map(|row| MainWindowRuntimeChatSnapshot {
                    sender: row.sender.clone(),
                    message: row.message.clone(),
                })
                .collect(),
            can_toggle_pause: state.playback.can_toggle_pause,
            can_seek: state.playback.can_seek,
            can_undo_seek: state.playback.can_undo_seek,
            can_set_offset: state.playback.can_set_offset,
            can_toggle_autoplay: state.playback.can_toggle_autoplay,
            can_adjust_autoplay_threshold: state.playback.can_adjust_autoplay_threshold,
            can_set_ready: state.playback.can_set_ready,
            can_set_others_ready: state.playback.can_set_others_ready,
            can_manage_playlist: state.playback.can_manage_playlist,
            playback_paused: state.playback_paused,
            autoplay_active: state.autoplay_active,
            autoplay_threshold: state.autoplay_threshold,
            autoplay_countdown_seconds: state.autoplay_countdown_seconds,
            user_offset_seconds: state.user_offset_seconds,
            show_playback_buttons: state.show_playback_buttons,
            show_autoplay_controls: state.show_autoplay_controls,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuActionShellItem {
    label: &'static str,
    enabled: bool,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuSectionShellState {
    title: &'static str,
    actions: Vec<MenuActionShellItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuDialogShellState {
    sections: Vec<MenuSectionShellState>,
    tls_prompt_expected: bool,
    update_notice_expected: bool,
    about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuActionRuntimeOverride {
    section_title: &'static str,
    action_label: &'static str,
    enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuDialogRuntimeSnapshot {
    action_overrides: Vec<MenuActionRuntimeOverride>,
    tls_prompt_expected: bool,
    update_notice_expected: bool,
    about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicServerBrowserRow {
    label: String,
    address: String,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicServerBrowserShellState {
    servers: Vec<PublicServerBrowserRow>,
    can_connect: bool,
    can_refresh: bool,
    can_add_custom_server: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublicServerBrowserRuntimeFlags {
    can_connect: bool,
    can_refresh: bool,
    can_add_custom_server: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaSearchDirectoryRow {
    path: String,
    is_selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct MediaSearchWorkflowShellState {
    directories: Vec<MediaSearchDirectoryRow>,
    can_browse_directories: bool,
    can_search_missing_media: bool,
    first_file_timeout_seconds: Option<f64>,
    search_timeout_seconds: Option<f64>,
    double_check_interval_seconds: Option<f64>,
    warning_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MediaSearchWorkflowRuntimeFlags {
    can_browse_directories: bool,
    can_search_missing_media: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct SyncplayGuiRuntimeSnapshot {
    active_view: GuiShellView,
    open_modal: Option<GuiShellModal>,
    main_window: MainWindowRuntimeSnapshot,
    public_servers: PublicServerBrowserShellState,
    media_search: MediaSearchWorkflowShellState,
    tls_prompt_expected: bool,
    update_notice_expected: bool,
    about_dialog_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiFeedbackRuntimeSnapshot {
    validation_issues: Vec<GuiValidationIssue>,
    notifications: Vec<GuiTransientNotification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiErrorRuntimeSnapshot {
    last_action_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct SyncplayGuiShellAppState {
    active_view: GuiShellView,
    open_modal: Option<GuiShellModal>,
    selection: GuiSelectionState,
    runtime_menu_action_overrides: Vec<MenuActionRuntimeOverride>,
    runtime_command_availability_override: GuiCommandAvailabilityRuntimeOverride,
    commands: GuiCommandAvailabilityState,
    pending_operation: Option<GuiPendingOperationState>,
    outgoing_chat_message: Option<String>,
    new_main_window_user_draft: String,
    new_playlist_entry_draft: String,
    focused_configuration_control: Option<GuiFocusedConfigurationControlState>,
    public_server_edit_session: Option<GuiPublicServerEditSessionState>,
    main_window_user_edit_session: Option<GuiMainWindowUserEditSessionState>,
    text_edit_session: Option<GuiTextEditSessionState>,
    playlist_text_edit_session: Option<GuiPlaylistTextEditSessionState>,
    playlist_url_edit_session: Option<GuiUrlEditSessionState>,
    media_url_edit_session: Option<GuiUrlEditSessionState>,
    controlled_room_create_session: Option<GuiControlledRoomCreateSessionState>,
    controller_auth_edit_session: Option<GuiControllerAuthEditSessionState>,
    room_history_edit_session: Option<GuiRoomHistoryEditSessionState>,
    update_check: GuiUpdateCheckState,
    runtime_validation_issues: Vec<GuiValidationIssue>,
    notifications: Vec<GuiTransientNotification>,
    validation: GuiValidationState,
    last_media_dialog_directory: Option<String>,
    playlist_undo_snapshot: Option<Vec<String>>,
    playlist_shuffle_nonce: u64,
    saved_configuration: StoredClientSettingsMvp,
    configuration: FirstRunConfigurationDialogDraft,
    main_window: MainWindowShellState,
    menus: MenuDialogShellState,
    public_servers: PublicServerBrowserShellState,
    media_search: MediaSearchWorkflowShellState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct GuiSelectionState {
    selected_main_window_user: Option<usize>,
    selected_main_window_playlist: Option<usize>,
    selected_menu_action: Option<(usize, usize)>,
    selected_media_search_directory: Option<usize>,
}

fn browser_is_url(value: &str) -> bool {
    value.contains("://")
}

fn browser_domain_from_url(value: &str) -> Option<String> {
    reqwest::Url::parse(value).ok().and_then(|url| {
        url.host_str()
            .map(|host| host.strip_prefix("www.").unwrap_or(host).to_owned())
    })
}

fn browser_parse_trustable_web_uri_host_and_path(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let authority_and_path = if let Some(rest) = value.strip_prefix("http://") {
        rest
    } else if let Some(rest) = value.strip_prefix("https://") {
        rest
    } else {
        return None;
    };
    if authority_and_path.is_empty() {
        return None;
    }
    let (authority, path_tail) = authority_and_path
        .split_once('/')
        .unwrap_or((authority_and_path, ""));
    if authority.is_empty() {
        return None;
    }
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, trimmed)| trimmed);
    if authority.is_empty() {
        return None;
    }
    let host = authority
        .split(':')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let path_with_query = if path_tail.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path_tail}")
    };
    let path = path_with_query
        .split(['?', '#'])
        .next()
        .unwrap_or("/")
        .to_owned();
    Some((host, path))
}

fn browser_trusted_domain_matches_host(host: &str, trusted_domain: &str) -> bool {
    if host == trusted_domain || host == format!("www.{trusted_domain}") {
        return true;
    }
    if !trusted_domain.contains('*') {
        return false;
    }
    let host_parts = host.split('.').collect::<Vec<_>>();
    let pattern_parts = trusted_domain.split('.').collect::<Vec<_>>();
    if host_parts.len() != pattern_parts.len() {
        return false;
    }
    host_parts
        .iter()
        .zip(pattern_parts.iter())
        .all(|(host_part, pattern_part)| {
            if *pattern_part == "*" {
                !host_part.is_empty()
            } else {
                host_part.eq_ignore_ascii_case(pattern_part)
            }
        })
}

fn browser_uri_is_trusted(
    uri: &str,
    only_switch_to_trusted_domains: bool,
    trusted_domains: &[String],
) -> bool {
    if !browser_is_url(uri) {
        return true;
    }
    let Some((host, path)) = browser_parse_trustable_web_uri_host_and_path(uri) else {
        return false;
    };
    if !only_switch_to_trusted_domains {
        return true;
    }
    trusted_domains.iter().any(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        let (trusted_domain, required_path_prefix) = entry.split_once('/').unwrap_or((entry, ""));
        let trusted_domain = trusted_domain.trim().to_ascii_lowercase();
        if trusted_domain.is_empty() || !browser_trusted_domain_matches_host(&host, &trusted_domain)
        {
            return false;
        }
        if required_path_prefix.is_empty() {
            return true;
        }
        path.starts_with(&format!("/{required_path_prefix}"))
    })
}

fn playlist_entries_from_multiline_text(value: &str) -> Vec<String> {
    value.lines().filter_map(normalized_editable_text).collect()
}

fn playlist_entries_multiline_text(entries: &[String]) -> String {
    entries.join("\n")
}

fn load_playlist_entries_from_path(path: &str) -> Result<Vec<String>, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read playlist file '{path}': {error}"))
        .map(|contents| {
            contents
                .lines()
                .filter_map(normalized_editable_text)
                .collect()
        })
}

fn save_playlist_entries_to_path(path: &str, entries: &[String]) -> Result<(), String> {
    std::fs::write(path, playlist_entries_multiline_text(entries))
        .map_err(|error| format!("Failed to save playlist file '{path}': {error}"))
}

fn playlist_next_shuffle_state(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state
}

fn shuffle_playlist_entries_in_place(entries: &mut [String], seed: u64) {
    if entries.len() <= 1 {
        return;
    }
    let mut state = seed;
    for index in (1..entries.len()).rev() {
        let random_value = playlist_next_shuffle_state(&mut state);
        let swap_index = (random_value as usize) % (index + 1);
        entries.swap(index, swap_index);
    }
}

fn browser_format_time(seconds: f64) -> String {
    let rounded = seconds.abs().round() as i64;
    let sign = if seconds.is_sign_negative() { "-" } else { "" };
    let days = rounded / 86_400;
    let hours = (rounded % 86_400) / 3_600;
    let minutes = (rounded % 3_600) / 60;
    let seconds = rounded % 60;
    if days > 0 {
        format!("{sign}{days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{minutes:02}:{seconds:02}")
    }
}

fn browser_number_from_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|number| number as f64))
        .or_else(|| value.as_u64().map(|number| number as f64))
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn browser_format_duration_label(value: Option<&Value>) -> String {
    let Some(seconds) = value.and_then(browser_number_from_value) else {
        return String::new();
    };
    format!("({})", browser_format_time(seconds))
}

fn browser_format_size_label(value: Option<&Value>) -> String {
    let Some(bytes) = value.and_then(browser_number_from_value) else {
        return String::new();
    };
    if bytes <= 0.0 {
        return "???".to_owned();
    }
    let megabytes = (bytes / 1_048_576.0).floor() as i64;
    format!("{megabytes} MB")
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct GuiCommandAvailabilityState {
    can_save_configuration: bool,
    can_reset_configuration: bool,
    can_reload_configuration: bool,
    can_connect_saved_server: bool,
    can_disconnect_session: bool,
    can_connect_public_server: bool,
    can_refresh_public_servers: bool,
    can_search_missing_media: bool,
    can_toggle_pause: bool,
    can_send_chat_message: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct GuiCommandAvailabilityRuntimeOverride {
    can_save_configuration: Option<bool>,
    can_reset_configuration: Option<bool>,
    can_reload_configuration: Option<bool>,
    can_connect_saved_server: Option<bool>,
    can_disconnect_session: Option<bool>,
    can_connect_public_server: Option<bool>,
    can_refresh_public_servers: Option<bool>,
    can_search_missing_media: Option<bool>,
    can_toggle_pause: Option<bool>,
    can_send_chat_message: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiCommandRuntimeSnapshot {
    command_availability: GuiCommandAvailabilityState,
    pending_operation: Option<GuiPendingOperationKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiFocusedConfigurationControlRuntimeSnapshot {
    section: String,
    label: String,
    activation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiPublicServerEditSessionRuntimeSnapshot {
    editing_index: Option<usize>,
    label_buffer: String,
    address_buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiMainWindowUserEditSessionRuntimeSnapshot {
    editing_index: usize,
    username_buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiTextEditSessionRuntimeSnapshot {
    section: String,
    label: String,
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiPlaylistTextEditSessionRuntimeSnapshot {
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiUrlEditSessionRuntimeSnapshot {
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiInteractionRuntimeSnapshot {
    selection: GuiSelectionState,
    selected_public_server_index: Option<usize>,
    focused_configuration_control: Option<GuiFocusedConfigurationControlRuntimeSnapshot>,
    public_server_edit_session: Option<GuiPublicServerEditSessionRuntimeSnapshot>,
    main_window_user_edit_session: Option<GuiMainWindowUserEditSessionRuntimeSnapshot>,
    text_edit_session: Option<GuiTextEditSessionRuntimeSnapshot>,
    playlist_text_edit_session: Option<GuiPlaylistTextEditSessionRuntimeSnapshot>,
    playlist_url_edit_session: Option<GuiUrlEditSessionRuntimeSnapshot>,
    media_url_edit_session: Option<GuiUrlEditSessionRuntimeSnapshot>,
}

impl GuiInteractionRuntimeSnapshot {
    fn from_shell_state(state: &SyncplayGuiShellAppState) -> Self {
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
struct GuiDraftRuntimeSnapshot {
    outgoing_chat_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiConfigurationDraftRuntimeSnapshot {
    settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiSavedConfigurationRuntimeSnapshot {
    settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiConfigurationRuntimeSnapshot {
    draft_settings: StoredClientSettingsMvp,
    saved_settings: StoredClientSettingsMvp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiSavedSessionConnectTarget {
    address: String,
    username: String,
    room: String,
    controlled_room_password_override: Option<String>,
}

trait GuiAppHost {
    type Output;

    fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output;
}

#[derive(Debug, Default)]
struct GuiWidgetEguiRenderer {
    stack: Vec<GuiWidgetNode>,
    root: Option<GuiWidgetNode>,
    actions: Vec<GuiShellAction>,
    close_requested: bool,
    playback_prompt_requested: Option<GuiPlaybackPromptKind>,
    selected_media_files: Option<Vec<String>>,
    dropped_files_request: Option<GuiDroppedFilesRequest>,
    playlist_drop_target_rect: Option<egui::Rect>,
    playlist_drop_target_hovered: bool,
    pending_completion_requested: bool,
    pending_cancel_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiPlaybackPromptKind {
    Seek,
    Offset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiDroppedFilesTarget {
    Window,
    Playlist,
}

impl GuiDroppedFilesTarget {
    fn parse(token: &str) -> Result<Self, String> {
        match token.trim() {
            "window" => Ok(Self::Window),
            "playlist" => Ok(Self::Playlist),
            other => Err(format!(
                "unknown dropped-files target {other:?}; expected 'window' or 'playlist'"
            )),
        }
    }

    fn load_into_shared_playlist(self, state: &SyncplayGuiShellAppState) -> bool {
        matches!(self, Self::Playlist) && state.shared_playlist_drop_target_available()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiDroppedFilesRequest {
    target: GuiDroppedFilesTarget,
    paths: Vec<String>,
}

impl GuiWidgetEguiRenderer {
    fn root(&self) -> Option<&GuiWidgetNode> {
        self.root.as_ref()
    }

    fn take_close_requested(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    fn take_playback_prompt_requested(&mut self) -> Option<GuiPlaybackPromptKind> {
        self.playback_prompt_requested.take()
    }

    fn take_selected_media_files(&mut self) -> Option<Vec<String>> {
        self.selected_media_files.take()
    }

    fn take_dropped_files_request(&mut self) -> Option<GuiDroppedFilesRequest> {
        self.dropped_files_request.take()
    }

    fn take_pending_completion_requested(&mut self) -> bool {
        std::mem::take(&mut self.pending_completion_requested)
    }

    fn take_pending_cancel_requested(&mut self) -> bool {
        std::mem::take(&mut self.pending_cancel_requested)
    }

    fn show(
        &mut self,
        ctx: &egui::Context,
        state: &SyncplayGuiShellAppState,
        show_manual_pending_controls: bool,
    ) -> Vec<GuiShellAction> {
        self.playlist_drop_target_rect = None;
        self.playlist_drop_target_hovered = false;
        self.dropped_files_request = None;
        if let Some(root) = self.root().cloned() {
            self.show_menu_bar(ctx, &root, state);
            self.show_modal_window(ctx, state);
            self.show_status_bar(ctx, &root, show_manual_pending_controls);
            self.show_navigation_panel(ctx, &root, state);
            self.show_active_surface(ctx, &root, state);
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Syncplay GUI");
                ui.label("No widget tree is currently available.");
            });
        }
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        self.dropped_files_request = Self::dropped_files_request_for_input(
            state,
            self.playlist_drop_target_hovered,
            self.playlist_drop_target_rect,
            ctx.input(|input| input.pointer.hover_pos()),
            dropped_files,
        );
        std::mem::take(&mut self.actions)
    }

    fn dropped_files_request_for_input(
        state: &SyncplayGuiShellAppState,
        playlist_drop_target_hovered: bool,
        playlist_drop_target_rect: Option<egui::Rect>,
        pointer_hover_pos: Option<egui::Pos2>,
        dropped_files: Vec<egui::DroppedFile>,
    ) -> Option<GuiDroppedFilesRequest> {
        let paths = dropped_files
            .iter()
            .filter_map(Self::dropped_file_path)
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return None;
        }
        let hovered_playlist_target = playlist_drop_target_hovered
            || playlist_drop_target_rect
                .zip(pointer_hover_pos)
                .is_some_and(|(rect, pointer)| rect.contains(pointer));
        let target = if hovered_playlist_target
            && GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(state)
        {
            GuiDroppedFilesTarget::Playlist
        } else {
            GuiDroppedFilesTarget::Window
        };
        Some(GuiDroppedFilesRequest { target, paths })
    }

    fn dropped_file_path(file: &egui::DroppedFile) -> Option<String> {
        if let Some(path) = file.path.as_ref() {
            return Some(path.to_string_lossy().into_owned());
        }
        normalized_editable_text(&file.name)
    }

    fn show_menu_bar(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(menus) = root.find("menus-root") else {
            return;
        };
        egui::TopBottomPanel::top("syncplay-native-menu-bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for section in &menus.children {
                    ui.menu_button(&section.label, |ui| {
                        self.render_menu_section(ui, section, state);
                    });
                }
            });
        });
    }

    fn show_modal_window(&mut self, ctx: &egui::Context, state: &SyncplayGuiShellAppState) {
        let Some(modal) = state.open_modal else {
            return;
        };
        let mut open = true;
        let mut close_clicked = false;
        egui::Window::new(Self::modal_window_title(modal))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                for line in Self::modal_body_lines(modal, state) {
                    ui.label(line);
                }
                if modal == GuiShellModal::UpdateNotice
                    && let Some(url) = state.update_check.url.as_deref()
                {
                    ui.hyperlink_to("Open update page", url);
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for (_, label, action) in Self::modal_actions(modal) {
                        if ui.button(label).clicked() {
                            self.actions.push(action);
                        }
                    }
                });
                ui.separator();
                if ui.button("Close").clicked() {
                    close_clicked = true;
                }
            });
        if !open || close_clicked {
            self.actions.push(GuiShellAction::CloseModal);
        }
    }

    fn show_status_bar(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        show_manual_pending_controls: bool,
    ) {
        let active_view = root
            .find("shell:active-view")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        let open_modal = root
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        let pending_operation = root
            .find("shell:pending-operation")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        egui::TopBottomPanel::bottom("syncplay-native-status-bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Syncplay GUI");
                ui.separator();
                ui.label(format!("view: {active_view}"));
                ui.separator();
                ui.label(format!("modal: {open_modal}"));
                ui.separator();
                ui.label(format!("pending: {pending_operation}"));
                if Self::should_show_manual_pending_controls(
                    pending_operation,
                    show_manual_pending_controls,
                ) {
                    ui.separator();
                    if ui.button("Complete").clicked() {
                        self.pending_completion_requested = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_cancel_requested = true;
                    }
                }
            });
        });
    }

    fn should_show_manual_pending_controls(
        pending_operation: &str,
        show_manual_pending_controls: bool,
    ) -> bool {
        show_manual_pending_controls && pending_operation != "(none)"
    }

    fn show_navigation_panel(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        egui::SidePanel::left("syncplay-native-navigation")
            .default_width(240.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Surfaces");
                ui.separator();
                for child in &root.children {
                    if Self::is_surface_node(child) {
                        let response =
                            ui.add(egui::Button::new(&child.label).selected(child.selected));
                        if response.clicked()
                            && let Some(action) = Self::action_for_surface_node(child)
                        {
                            self.actions.push(action);
                        }
                    }
                }
                if let Some(quick_actions) = root.find("shell:quick-actions") {
                    ui.separator();
                    ui.heading("Quick Actions");
                    for action in &quick_actions.children {
                        self.render_leaf(ui, action, state);
                    }
                }
                Self::render_sidebar_list_branch(ui, root.find("shell:commands"), "Commands");
                Self::render_sidebar_list_branch(ui, root.find("shell:validation"), "Validation");
                if let Some(notifications) = root.find("shell:notifications") {
                    ui.separator();
                    ui.heading("Notifications");
                    if notifications.children.is_empty() {
                        ui.label("No transient notifications.");
                    } else {
                        for notification in &notifications.children {
                            if ui
                                .selectable_label(false, Self::display_text(notification))
                                .clicked()
                                && let Some(action) = Self::action_for_list_item_node(notification)
                            {
                                self.actions.push(action);
                            }
                        }
                    }
                }
            });
    }

    fn render_sidebar_list_branch(
        ui: &mut egui::Ui,
        branch: Option<&GuiWidgetNode>,
        heading: &str,
    ) {
        let Some(branch) = branch else {
            return;
        };
        ui.separator();
        ui.heading(heading);
        if branch.children.is_empty() {
            ui.label("No items.");
        } else {
            for item in &branch.children {
                ui.label(Self::display_text(item));
            }
        }
    }

    fn show_active_surface(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let active_surface = root
            .children
            .iter()
            .find(|node| Self::is_surface_node(node) && node.selected)
            .or_else(|| {
                root.children
                    .iter()
                    .find(|node| Self::is_surface_node(node))
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(active_surface) = active_surface {
                    ui.heading(&active_surface.label);
                    ui.separator();
                    self.render_node(ui, active_surface, state);
                } else {
                    ui.heading(&root.label);
                    ui.label("No active surface is currently selected.");
                }
            });
        });
    }

    fn render_menu_section(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        for child in &node.children {
            if child.children.is_empty() {
                self.render_leaf(ui, child, state);
            } else {
                ui.menu_button(&child.label, |ui| {
                    self.render_menu_section(ui, child, state);
                });
            }
        }
    }

    fn render_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::Panel => {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&node.label);
                        if node.selected {
                            ui.label(egui::RichText::new("active").small().strong());
                        }
                        if !node.enabled {
                            ui.label(egui::RichText::new("disabled").small());
                        }
                    });
                    for child in &node.children {
                        self.render_node(ui, child, state);
                    }
                });
            }
            GuiWidgetKind::List => {
                let response = egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.strong(&node.label);
                    if node.children.is_empty() {
                        ui.label("No items.");
                    } else {
                        for child in &node.children {
                            self.render_node(ui, child, state);
                        }
                    }
                });
                if node.id == "main-window:playlist" {
                    self.playlist_drop_target_rect = Some(response.response.rect);
                    self.playlist_drop_target_hovered = response.response.hovered();
                }
            }
            _ => self.render_leaf(ui, node, state),
        }
    }

    fn render_leaf(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::TextInput
            | GuiWidgetKind::TextArea
            | GuiWidgetKind::PasswordInput
            | GuiWidgetKind::NumericInput => {
                if node.kind == GuiWidgetKind::TextArea {
                    self.render_text_area(ui, node, state);
                } else {
                    self.render_text_input(ui, node, state);
                }
            }
            GuiWidgetKind::Select => self.render_select(ui, node, state),
            GuiWidgetKind::Checkbox => {
                let mut checked = matches!(node.value.as_deref(), Some("yes" | "true"));
                let response =
                    ui.add_enabled(node.enabled, egui::Checkbox::new(&mut checked, &node.label));
                if response.changed()
                    && let Some(action) = Self::action_for_checkbox_node(state, node, checked)
                {
                    self.actions.push(action);
                }
            }
            GuiWidgetKind::Button => {
                if ui
                    .add_enabled(node.enabled, egui::Button::new(Self::display_text(node)))
                    .clicked()
                {
                    if node.id == "shell:quick:open-media-file"
                        || Self::is_open_media_file_menu_action(state, node)
                    {
                        self.selected_media_files = Self::pick_media_files(state);
                    } else if Self::is_exit_menu_action(state, node) {
                        self.close_requested = true;
                    } else if let Some(actions) = Self::direct_menu_actions(state, node) {
                        self.actions.extend(actions);
                    } else {
                        self.actions
                            .extend(Self::actions_for_clicked_button(state, node));
                    }
                }
            }
            GuiWidgetKind::ListItem => {
                ui.add_enabled_ui(node.enabled, |ui| {
                    if ui
                        .selectable_label(node.selected, Self::display_text(node))
                        .clicked()
                        && let Some(action) = Self::action_for_list_item_node(node)
                    {
                        self.actions.push(action);
                    }
                });
            }
            GuiWidgetKind::ReadOnly | GuiWidgetKind::Status => {
                if Self::should_render_combined_status_label(node) {
                    ui.label(Self::display_text(node));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(&node.label).strong());
                        ui.label(node.value.as_deref().unwrap_or("(none)"));
                    });
                }
            }
            GuiWidgetKind::Panel | GuiWidgetKind::List => {}
        }
    }

    fn render_text_input(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_else(|| "(none)".to_owned());
        ui.horizontal(|ui| {
            ui.label(&node.label);
            let response = ui.add_enabled(
                node.enabled,
                egui::TextEdit::singleline(&mut value)
                    .password(matches!(node.kind, GuiWidgetKind::PasswordInput))
                    .desired_width(260.0),
            );
            let submitted =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if let Some(actions) = Self::actions_for_text_input_node(
                state,
                node,
                &value,
                response.changed(),
                submitted,
            ) {
                self.actions.extend(actions);
            }
        });
    }

    fn render_text_area(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_default();
        ui.vertical(|ui| {
            ui.label(&node.label);
            let response = ui.add_enabled(
                node.enabled,
                egui::TextEdit::multiline(&mut value)
                    .desired_width(360.0)
                    .desired_rows(6),
            );
            if let Some(actions) =
                Self::actions_for_text_input_node(state, node, &value, response.changed(), false)
            {
                self.actions.extend(actions);
            }
        });
    }

    fn render_select(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_default();
        let previous = value.clone();
        let options = Self::configuration_select_options_for_node(state, node)
            .unwrap_or_else(|| vec![previous.clone()]);
        ui.horizontal(|ui| {
            ui.label(&node.label);
            ui.add_enabled_ui(node.enabled, |ui| {
                egui::ComboBox::from_id_salt(&node.id)
                    .selected_text(if value.is_empty() { "(unset)" } else { &value })
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for option in &options {
                            ui.selectable_value(&mut value, option.clone(), option);
                        }
                    });
            });
        });
        if value != previous
            && let Some(actions) =
                Self::actions_for_text_input_node(state, node, &value, true, false)
        {
            self.actions.extend(actions);
        }
    }

    fn modal_window_title(modal: GuiShellModal) -> &'static str {
        match modal {
            GuiShellModal::TlsCertificatePrompt => "TLS Certificate Prompt",
            GuiShellModal::UpdateNotice => "Update Notice",
            GuiShellModal::About => "About Syncplay",
        }
    }

    fn modal_body_lines(modal: GuiShellModal, state: &SyncplayGuiShellAppState) -> Vec<String> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                "A TLS certificate prompt is active for the current connection.".to_owned(),
                "Trust the certificate for this session or reject it to keep the warning visible."
                    .to_owned(),
            ],
            GuiShellModal::UpdateNotice => state.update_check.body_lines(),
            GuiShellModal::About => vec![
                "The reducer reports that the About dialog is open.".to_owned(),
                "This modal now routes into the existing help and update actions.".to_owned(),
            ],
        }
    }

    fn modal_actions(modal: GuiShellModal) -> Vec<(&'static str, &'static str, GuiShellAction)> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                (
                    "shell:modal:tls:trust",
                    "Trust Certificate",
                    GuiShellAction::TrustTlsCertificatePrompt,
                ),
                (
                    "shell:modal:tls:reject",
                    "Reject Certificate",
                    GuiShellAction::RejectTlsCertificatePrompt,
                ),
                (
                    "shell:modal:tls:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
            ],
            GuiShellModal::UpdateNotice => vec![
                (
                    "shell:modal:update:dismiss",
                    "Dismiss Notice",
                    GuiShellAction::DismissUpdateNotice,
                ),
                (
                    "shell:modal:update:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
                (
                    "shell:modal:update:check-again",
                    "Check Again",
                    GuiShellAction::AnnounceUpdateNoticeAvailable,
                ),
            ],
            GuiShellModal::About => vec![
                (
                    "shell:modal:about:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
                (
                    "shell:modal:about:update",
                    "Check for Updates",
                    GuiShellAction::AnnounceUpdateNoticeAvailable,
                ),
            ],
        }
    }

    fn display_text(node: &GuiWidgetNode) -> String {
        match node.value.as_deref() {
            Some(value) if !value.is_empty() => format!("{}: {}", node.label, value),
            _ => node.label.clone(),
        }
    }

    fn should_render_combined_status_label(node: &GuiWidgetNode) -> bool {
        node.id.starts_with("media-search:timing:")
            || node.id.starts_with("shell:command:")
            || node.id.starts_with("shell:validation:")
    }

    fn action_for_surface_node(node: &GuiWidgetNode) -> Option<GuiShellAction> {
        let view = match node.id.as_str() {
            "configuration-root" => GuiShellView::Configuration,
            "main-window-root" => GuiShellView::MainWindow,
            "menus-root" => GuiShellView::MenusAndDialogs,
            "public-servers-root" => GuiShellView::PublicServers,
            "media-search-root" => GuiShellView::MediaSearch,
            _ => return None,
        };
        Some(GuiShellAction::SwitchView(view))
    }

    fn actions_for_button_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Vec<GuiShellAction> {
        if let Some(room_index) = Self::main_window_browser_room_action_index(&node.id, "join") {
            return state
                .main_window
                .rooms
                .get(room_index)
                .map(|room| vec![GuiShellAction::JoinMainWindowRoom(room.room_name.clone())])
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "open") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.clone())
                .map(|target| vec![GuiShellAction::RequestMainWindowUserMediaOpen(target)])
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "folder") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.clone())
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
                        target,
                    )]
                })
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "ready") {
            return state
                .main_window
                .users
                .get(user_index)
                .filter(|user| state.can_request_main_window_user_ready_change(user))
                .map(|user| {
                    vec![GuiShellAction::RequestMainWindowUserReady {
                        username: user.username.clone(),
                        ready: !user.is_ready,
                    }]
                })
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "trust") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.as_deref())
                .and_then(browser_domain_from_url)
                .map(|domain| vec![GuiShellAction::AddTrustedDomain(domain)])
                .unwrap_or_default();
        }
        if node.id == "main-window:playlist:add-files" {
            return Self::pick_media_files(state)
                .map(GuiShellAction::AppendSharedPlaylistEntries)
                .into_iter()
                .collect();
        }
        if matches!(
            node.id.as_str(),
            "main-window:playlist:load" | "main-window:playlist:load-shuffle"
        ) {
            let Some(path) = Self::pick_playlist_load_file(state) else {
                return Vec::new();
            };
            return match load_playlist_entries_from_path(&path) {
                Ok(entries) => vec![GuiShellAction::LoadSharedPlaylistFromFile {
                    path,
                    entries,
                    shuffled: node.id == "main-window:playlist:load-shuffle",
                }],
                Err(error) => vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(error),
                ],
            };
        }
        if node.id == "main-window:playlist:save" {
            let Some(path) = Self::pick_playlist_save_file(state) else {
                return Vec::new();
            };
            let entries = state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.clone())
                .collect::<Vec<_>>();
            return match save_playlist_entries_to_path(&path, &entries) {
                Ok(()) => vec![GuiShellAction::SaveSharedPlaylistToFile(path)],
                Err(error) => vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(error),
                ],
            };
        }
        match node.id.as_str() {
            "config-command:edit-room-history" => vec![GuiShellAction::BeginRoomHistoryEdit],
            "config-command:connect" => vec![GuiShellAction::BeginSavedServerConnect],
            "config-command:disconnect" => vec![GuiShellAction::BeginSessionDisconnect],
            "config-command:save" => vec![GuiShellAction::BeginConfigurationSave],
            "config-command:reset" => vec![GuiShellAction::BeginConfigurationReset],
            "config-command:reload" => vec![GuiShellAction::BeginConfigurationReload],
            "config-command:clear-gui-data" => vec![GuiShellAction::BeginClearGuiData],
            "main-window:connection:connect" => vec![GuiShellAction::BeginSavedServerConnect],
            "main-window:connection:disconnect" => {
                vec![GuiShellAction::BeginSessionDisconnect]
            }
            "main-window:control:play" => vec![GuiShellAction::BeginPlaybackResume],
            "main-window:control:pause" => vec![GuiShellAction::BeginPlaybackPause],
            "main-window:control:toggle-pause" => vec![GuiShellAction::BeginPlaybackPauseToggle],
            "main-window:control:seek" => vec![GuiShellAction::RequestSeekPrompt],
            "main-window:control:undo-seek" => vec![GuiShellAction::RequestPlaybackUndoSeek],
            "main-window:control:set-offset" => vec![GuiShellAction::RequestOffsetPrompt],
            "main-window:control:autoplay-threshold-down" => state
                .main_window
                .autoplay_threshold
                .checked_sub(1)
                .filter(|threshold| *threshold >= 2)
                .map(GuiShellAction::AnnounceAutoplayThreshold)
                .into_iter()
                .collect(),
            "main-window:control:autoplay-threshold-up" => {
                vec![GuiShellAction::AnnounceAutoplayThreshold(
                    state
                        .main_window
                        .autoplay_threshold
                        .saturating_add(1)
                        .min(99),
                )]
            }
            "main-window:room:set" => {
                vec![GuiShellAction::SetMainWindowRoom(
                    Self::main_window_room_draft(state),
                )]
            }
            "main-window:room:join" => {
                vec![GuiShellAction::JoinMainWindowRoom(
                    Self::main_window_room_draft(state),
                )]
            }
            "main-window:room:leave" => vec![GuiShellAction::LeaveMainWindowRoom],
            "main-window:user:add" => vec![GuiShellAction::CommitNewMainWindowUser],
            "main-window:user:toggle-ready" => {
                vec![GuiShellAction::ToggleSelectedMainWindowUserReady]
            }
            "main-window:user:toggle-controller" => {
                vec![GuiShellAction::ToggleSelectedMainWindowUserController]
            }
            "main-window:user:edit" => vec![GuiShellAction::BeginEditSelectedMainWindowUser],
            "main-window:user:remove" => vec![GuiShellAction::RemoveSelectedMainWindowUser],
            "main-window:user-edit:commit" => vec![GuiShellAction::CommitMainWindowUserEdit],
            "main-window:user-edit:cancel" => vec![GuiShellAction::CancelMainWindowUserEdit],
            "main-window:playlist:add" => vec![GuiShellAction::CommitNewPlaylistEntry],
            "main-window:playlist:up" => vec![GuiShellAction::MoveSelectedMainWindowPlaylistUp],
            "main-window:playlist:down" => vec![GuiShellAction::MoveSelectedMainWindowPlaylistDown],
            "main-window:playlist:remove" => vec![GuiShellAction::RemoveSelectedMainWindowPlaylist],
            "main-window:playlist:add-url" => vec![GuiShellAction::BeginSharedPlaylistUrlEdit],
            "main-window:playlist:open-url" => vec![GuiShellAction::BeginMediaUrlEdit],
            "main-window:playlist:open-selected" => state
                .selected_shared_playlist_entry()
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserMediaOpen(
                        target.to_owned(),
                    )]
                })
                .unwrap_or_default(),
            "main-window:playlist:open-selected-folder" => state
                .selected_shared_playlist_entry()
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
                        target.to_owned(),
                    )]
                })
                .unwrap_or_default(),
            "main-window:playlist:trust-selected" => state
                .selected_shared_playlist_entry()
                .and_then(browser_domain_from_url)
                .map(|domain| vec![GuiShellAction::AddTrustedDomain(domain)])
                .unwrap_or_default(),
            "main-window:playlist:shuffle-remaining" => {
                vec![GuiShellAction::ShuffleRemainingSharedPlaylist]
            }
            "main-window:playlist:shuffle-entire" => {
                vec![GuiShellAction::ShuffleEntireSharedPlaylist]
            }
            "main-window:playlist:undo" => vec![GuiShellAction::UndoSharedPlaylistChange],
            "main-window:playlist:edit" => vec![GuiShellAction::BeginSharedPlaylistTextEdit],
            "main-window:playlist-edit:commit" => {
                let entries = state
                    .playlist_text_edit_session
                    .as_ref()
                    .map(|session| playlist_entries_from_multiline_text(&session.buffer))
                    .unwrap_or_default();
                vec![
                    GuiShellAction::ReplaceSharedPlaylistEntries(entries),
                    GuiShellAction::CancelSharedPlaylistTextEdit,
                ]
            }
            "main-window:playlist-edit:cancel" => {
                vec![GuiShellAction::CancelSharedPlaylistTextEdit]
            }
            "main-window:playlist-url-edit:commit" => {
                let entries = state
                    .playlist_url_edit_session
                    .as_ref()
                    .map(|session| playlist_entries_from_multiline_text(&session.buffer))
                    .unwrap_or_default();
                vec![
                    GuiShellAction::AppendSharedPlaylistEntries(entries),
                    GuiShellAction::CancelSharedPlaylistUrlEdit,
                ]
            }
            "main-window:playlist-url-edit:cancel" => {
                vec![GuiShellAction::CancelSharedPlaylistUrlEdit]
            }
            "main-window:media-url-edit:commit" => state
                .media_url_edit_session
                .as_ref()
                .and_then(|session| normalized_editable_text(&session.buffer))
                .map(|target| {
                    vec![
                        GuiShellAction::RequestMainWindowUserMediaOpen(target),
                        GuiShellAction::CancelMediaUrlEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:media-url-edit:cancel" => vec![GuiShellAction::CancelMediaUrlEdit],
            "main-window:controlled-room-create:commit" => state
                .controlled_room_create_session
                .as_ref()
                .and_then(|session| {
                    let room_name =
                        controlled_room_base_name_legacy_compatible(&session.room_buffer);
                    normalized_editable_text(&room_name)
                })
                .map(|room| {
                    vec![
                        GuiShellAction::RequestControllerAuth {
                            room,
                            password: generate_room_password_legacy_compatible(),
                        },
                        GuiShellAction::CancelCreateControlledRoomEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:controlled-room-create:cancel" => {
                vec![GuiShellAction::CancelCreateControlledRoomEdit]
            }
            "main-window:controller-auth:commit" => state
                .controller_auth_edit_session
                .as_ref()
                .filter(|session| normalized_editable_text(&session.password_buffer).is_some())
                .map(|session| {
                    vec![
                        GuiShellAction::RequestControllerAuth {
                            room: session.room_name.clone(),
                            password: session.password_buffer.clone(),
                        },
                        GuiShellAction::CancelControllerAuthEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:controller-auth:cancel" => {
                vec![GuiShellAction::CancelControllerAuthEdit]
            }
            "main-window:control:set-ready" => {
                let local_user_ready = state
                    .main_window
                    .users
                    .iter()
                    .find(|user| user.is_self)
                    .map(|user| user.is_ready)
                    .unwrap_or(false);
                vec![if local_user_ready {
                    GuiShellAction::AnnounceLocalUserNotReady
                } else {
                    GuiShellAction::AnnounceLocalUserReady
                }]
            }
            "public-servers:command:connect" => {
                vec![GuiShellAction::BeginSelectedPublicServerConnect]
            }
            "public-servers:command:refresh" => vec![GuiShellAction::BeginPublicServerRefresh],
            "public-servers:command:add-custom" => vec![GuiShellAction::BeginAddPublicServer],
            "public-servers:command:edit" => vec![GuiShellAction::BeginEditSelectedPublicServer],
            "public-servers:command:remove" => vec![GuiShellAction::RemoveSelectedPublicServer],
            "public-servers:edit:commit" => vec![GuiShellAction::CommitPublicServerEdit],
            "public-servers:edit:cancel" => vec![GuiShellAction::CancelPublicServerEdit],
            "media-search:directory:up" => vec![GuiShellAction::MoveSelectedMediaSearchDirectoryUp],
            "media-search:directory:down" => {
                vec![GuiShellAction::MoveSelectedMediaSearchDirectoryDown]
            }
            "media-search:directory:remove" => {
                vec![GuiShellAction::RemoveSelectedMediaSearchDirectory]
            }
            "media-search:command:search" => vec![GuiShellAction::BeginMissingMediaSearch],
            "room-history:edit:commit" => vec![GuiShellAction::CommitRoomHistoryEdit],
            "room-history:edit:cancel" => vec![GuiShellAction::CancelRoomHistoryEdit],
            "shell:modal:close" => vec![GuiShellAction::CloseModal],
            "shell:modal:update:dismiss" => vec![GuiShellAction::DismissUpdateNotice],
            "shell:modal:update:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:update:check-again" => vec![GuiShellAction::BeginUpdateCheck {
                user_initiated: true,
            }],
            "shell:modal:tls:trust" => vec![GuiShellAction::TrustTlsCertificatePrompt],
            "shell:modal:tls:reject" => vec![GuiShellAction::RejectTlsCertificatePrompt],
            "shell:modal:tls:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:about:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:about:update" => vec![GuiShellAction::BeginUpdateCheck {
                user_initiated: true,
            }],
            _ => {
                if let Some((section_index, action_index)) = Self::menu_action_identity(node) {
                    vec![
                        GuiShellAction::SelectMenuAction {
                            section_index,
                            action_index,
                        },
                        GuiShellAction::TriggerSelectedMenuAction,
                    ]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn actions_for_clicked_button(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Vec<GuiShellAction> {
        match node.id.as_str() {
            "media-search:command:browse" => Self::actions_for_media_search_browse_click(state),
            _ => Self::actions_for_button_node(state, node),
        }
    }

    fn actions_for_media_search_browse_click(
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::pick_media_search_directory(state)
            .map(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed)
            .into_iter()
            .collect()
    }

    fn is_open_media_file_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "File", "Open Media File")
    }

    fn is_exit_menu_action(state: &SyncplayGuiShellAppState, node: &GuiWidgetNode) -> bool {
        Self::matches_menu_action(state, node, "File", "Exit")
    }

    fn direct_menu_actions(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<GuiShellAction>> {
        let actions = if Self::matches_menu_action(state, node, "Playback", "Seek") {
            vec![GuiShellAction::RequestSeekPrompt]
        } else if Self::matches_menu_action(state, node, "Playback", "Undo Seek") {
            vec![GuiShellAction::RequestPlaybackUndoSeek]
        } else if Self::matches_menu_action(state, node, "Advanced", "Set Offset") {
            vec![GuiShellAction::RequestOffsetPrompt]
        } else {
            return None;
        };
        Some(actions)
    }

    fn is_seek_menu_action(state: &SyncplayGuiShellAppState, node: &GuiWidgetNode) -> bool {
        Self::matches_menu_action(state, node, "Playback", "Seek")
    }

    fn matches_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        section_title: &str,
        action_label: &str,
    ) -> bool {
        let Some((section_index, action_index)) = Self::menu_action_identity(node) else {
            return false;
        };
        let Some(section) = state.menus.sections.get(section_index) else {
            return false;
        };
        let Some(action) = section.actions.get(action_index) else {
            return false;
        };
        section.title == section_title && action.label == action_label
    }

    fn pick_media_files(state: &SyncplayGuiShellAppState) -> Option<Vec<String>> {
        if let Some(paths) = Self::media_file_pick_override_paths_from_lookup(&env_trimmed) {
            return Some(paths);
        }
        let mut dialog = FileDialog::new().set_title("Select Media File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_files().map(|paths| {
            paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        })
    }

    fn media_file_pick_override_paths_from_lookup<F>(lookup: &F) -> Option<Vec<String>>
    where
        F: Fn(&str) -> Option<String>,
    {
        let paths = lookup("SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS")?
            .split('|')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if paths.is_empty() { None } else { Some(paths) }
    }

    fn pick_playlist_load_file(state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(path) = Self::playlist_load_override_path_from_lookup(&env_trimmed) {
            return Some(path);
        }
        let mut dialog = FileDialog::new().set_title("Load Playlist From File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .add_filter("playlist", &["txt", "m3u", "m3u8"])
            .pick_file()
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn playlist_load_override_path_from_lookup<F>(lookup: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        lookup("SYNCPLAY_GUI_TEST_LOAD_PLAYLIST_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn pick_playlist_save_file(state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(path) = Self::playlist_save_override_path_from_lookup(&env_trimmed) {
            return Some(path);
        }
        let mut dialog = FileDialog::new().set_title("Save Playlist To File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .add_filter("playlist", &["txt", "m3u", "m3u8"])
            .save_file()
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn playlist_save_override_path_from_lookup<F>(lookup: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        lookup("SYNCPLAY_GUI_TEST_SAVE_PLAYLIST_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn pick_media_search_directory(state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(path) = Self::media_search_browse_override_path_from_lookup(&env_trimmed) {
            return Some(path);
        }
        let mut dialog = FileDialog::new().set_title("Select Media Search Directory");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .pick_folder()
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn media_search_browse_override_path_from_lookup<F>(lookup: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        lookup("SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn media_search_dialog_start_directory(state: &SyncplayGuiShellAppState) -> Option<&str> {
        state.last_media_dialog_directory.as_deref().or_else(|| {
            state
                .selection
                .selected_media_search_directory
                .and_then(|index| state.media_search.directories.get(index))
                .or_else(|| state.media_search.directories.first())
                .map(|row| row.path.as_str())
        })
    }

    fn action_for_list_item_node(node: &GuiWidgetNode) -> Option<GuiShellAction> {
        Self::parse_index_suffix(&node.id, "main-window:user:")
            .map(GuiShellAction::SelectMainWindowUser)
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "main-window:playlist:")
                    .map(GuiShellAction::SelectMainWindowPlaylist)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "public-servers:row:")
                    .map(GuiShellAction::SelectPublicServer)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "media-search:directory:")
                    .map(GuiShellAction::SelectMediaSearchDirectory)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "shell:notification:")
                    .map(GuiShellAction::DismissTransientNotification)
            })
    }

    fn action_for_checkbox_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        value: bool,
    ) -> Option<GuiShellAction> {
        if node.id == "main-window:control:autoplay-toggle" {
            return Some(GuiShellAction::AnnounceAutoplayState(value));
        }
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Checkbox {
            return None;
        }
        Some(GuiShellAction::EditConfigurationBool {
            section,
            label,
            value,
        })
    }

    fn actions_for_text_input_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        value: &str,
        changed: bool,
        submitted: bool,
    ) -> Option<Vec<GuiShellAction>> {
        if node.id == "main-window:chat-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
                    GuiDraftRuntimeSnapshot {
                        outgoing_chat_message: normalized_editable_text(value),
                    },
                ));
            }
            if submitted {
                actions.push(GuiShellAction::BeginLocalChatSend(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:room-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::EditConfigurationText {
                    section: "Connection",
                    label: "Room",
                    value: value.to_owned(),
                });
            }
            if submitted && normalized_editable_text(value).is_some() {
                actions.push(GuiShellAction::JoinMainWindowRoom(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:user:new" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateNewMainWindowUserDraft(
                    value.to_owned(),
                ));
            }
            if submitted && normalized_editable_text(value).is_some() {
                actions.push(GuiShellAction::CommitNewMainWindowUser);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:playlist:new" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateNewPlaylistEntryDraft(
                    value.to_owned(),
                ));
            }
            if submitted && normalized_editable_text(value).is_some() {
                actions.push(GuiShellAction::CommitNewPlaylistEntry);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "room-history:edit:entries" {
            return changed.then(|| vec![GuiShellAction::UpdateRoomHistoryEdit(value.to_owned())]);
        }

        if node.id == "main-window:playlist-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:playlist-url-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:media-url-edit:text" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateMediaUrlEdit(value.to_owned()));
            }
            if submitted && let Some(target) = normalized_editable_text(value) {
                actions.push(GuiShellAction::RequestMainWindowUserMediaOpen(target));
                actions.push(GuiShellAction::CancelMediaUrlEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controlled-room-create:room" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateCreateControlledRoomEdit(
                    value.to_owned(),
                ));
            }
            if submitted {
                let room_name = controlled_room_base_name_legacy_compatible(value);
                if let Some(room_name) = normalized_editable_text(&room_name) {
                    actions.push(GuiShellAction::RequestControllerAuth {
                        room: room_name,
                        password: generate_room_password_legacy_compatible(),
                    });
                    actions.push(GuiShellAction::CancelCreateControlledRoomEdit);
                }
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controller-auth:password" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateControllerAuthPasswordEdit(
                    value.to_owned(),
                ));
            }
            if submitted
                && let Some(session) = state.controller_auth_edit_session.as_ref()
                && normalized_editable_text(value).is_some()
            {
                actions.push(GuiShellAction::RequestControllerAuth {
                    room: session.room_name.clone(),
                    password: value.to_owned(),
                });
                actions.push(GuiShellAction::CancelControllerAuthEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if let Some((section, label, kind)) = Self::configuration_control_identity(state, node) {
            if matches!(
                kind,
                GuiDialogControlKind::TextInput
                    | GuiDialogControlKind::PasswordInput
                    | GuiDialogControlKind::NumericInput
                    | GuiDialogControlKind::Select
            ) && changed
            {
                return Some(vec![GuiShellAction::EditConfigurationText {
                    section,
                    label,
                    value: value.to_owned(),
                }]);
            }
            return None;
        }

        let mut actions = Vec::new();
        match node.id.as_str() {
            "public-servers:edit:label" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditLabel(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "public-servers:edit:address" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditAddress(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "main-window:user-edit:username" => {
                if changed {
                    actions.push(GuiShellAction::UpdateMainWindowUserEdit(value.to_owned()));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitMainWindowUserEdit);
                }
            }
            _ => {}
        }
        (!actions.is_empty()).then_some(actions)
    }

    fn configuration_control_identity(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<(&'static str, &'static str, GuiDialogControlKind)> {
        let identity = node.id.strip_prefix("config:")?;
        let (section, label) = identity.split_once(':')?;
        state.configuration.control_identity(section, label)
    }

    fn menu_action_identity(node: &GuiWidgetNode) -> Option<(usize, usize)> {
        let identity = node.id.strip_prefix("menus:action:")?;
        let (section_index, action_index) = identity.split_once(':')?;
        Some((section_index.parse().ok()?, action_index.parse().ok()?))
    }

    fn configuration_select_options_for_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<String>> {
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Select {
            return None;
        }
        Some(match (section, label) {
            ("Readiness", "Unpause Action") => [
                "IfAlreadyReady",
                "IfOthersReady",
                "IfMinUsersReady",
                "Always",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            ("Readiness", "Autoplay Min Users") => {
                let mut options = ["app-default", "0", "1", "2", "3", "4", "5"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if let Some(value) = node.value.as_ref()
                    && !value.is_empty()
                    && !options.iter().any(|option| option == value)
                {
                    options.push(value.clone());
                }
                options
            }
            ("Privacy", "Filename Privacy") | ("Privacy", "Filesize Privacy") => {
                ["SendRaw", "SendHashed", "DoNotSend"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            }
            ("System", "Language") => SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY
                .split('/')
                .map(str::to_owned)
                .collect(),
            _ => return None,
        })
    }

    fn main_window_room_draft(state: &SyncplayGuiShellAppState) -> String {
        state
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default()
            .to_owned()
    }

    fn parse_index_suffix(id: &str, prefix: &str) -> Option<usize> {
        id.strip_prefix(prefix)?.parse().ok()
    }

    fn main_window_browser_room_action_index(id: &str, action: &str) -> Option<usize> {
        let identity = id.strip_prefix("main-window:room-group:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    fn main_window_browser_user_action_index(id: &str, action: &str) -> Option<usize> {
        let identity = id.strip_prefix("main-window:user:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    fn is_surface_node(node: &GuiWidgetNode) -> bool {
        matches!(
            node.id.as_str(),
            "configuration-root"
                | "main-window-root"
                | "public-servers-root"
                | "media-search-root"
                | "menus-root"
        )
    }
}

impl GuiWidgetRenderer for GuiWidgetEguiRenderer {
    fn begin_node(&mut self, node: &GuiWidgetNode, _depth: usize) {
        let mut shallow_node = node.clone();
        shallow_node.children.clear();
        self.stack.push(shallow_node);
    }

    fn end_node(&mut self, _node: &GuiWidgetNode, _depth: usize) {
        let Some(completed_node) = self.stack.pop() else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(completed_node);
        } else {
            self.root = Some(completed_node);
        }
    }
}

trait GuiNativeRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool;

    fn drain_runtime_actions(&mut self) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_open_media_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction>;

    fn actions_for_selected_media_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
    ) -> Vec<GuiShellAction> {
        self.actions_for_open_media_files(state, paths, state.shared_playlist_events_enabled())
    }

    fn actions_for_dropped_files(
        &mut self,
        state: &SyncplayGuiShellAppState,
        request: GuiDroppedFilesRequest,
    ) -> Vec<GuiShellAction> {
        self.actions_for_open_media_files(
            state,
            request.paths,
            request.target.load_into_shared_playlist(state),
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
        _state: &SyncplayGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _target: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_join(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _room: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_room_leave(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_local_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_main_window_user_readiness_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _username: String,
        _ready: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_controller_auth_request(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _room: String,
        _password: String,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_commit(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _entry: String,
        _select_after_queue: bool,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_selection_change(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_entry_removal(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _index: usize,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_reorder(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        _playlist: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_undo(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_remaining(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_playlist_shuffle_entire(
        &mut self,
        _state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction>;

    fn actions_for_pending_cancel(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction>;
}

trait GuiNativeRuntimePump {
    fn pump(&mut self, state: &SyncplayGuiShellAppState);
}

#[path = "live_python_interop.rs"]
pub(crate) mod live_python_interop;
#[path = "semantic_driver.rs"]
pub(crate) mod semantic_driver;
#[path = "semantic_smoke.rs"]
pub(crate) mod semantic_smoke;
#[cfg(test)]
use self::semantic_driver::GuiSemanticStep;
use self::semantic_smoke::run_syncplay_gui_semantic_cli_from_env;
#[cfg(test)]
use self::semantic_smoke::{
    gui_semantic_output_format_from_lookup, gui_semantic_scenario_name_from_lookup,
    gui_semantic_scenario_named, gui_semantic_scenario_names,
    run_gui_semantic_scenario_from_lookup, run_gui_semantic_scenario_named,
};

trait GuiQueuedRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SyncplayGuiShellAppState);
}

#[derive(Default)]
struct GuiNoopRuntimePump;

impl GuiNativeRuntimePump for GuiNoopRuntimePump {
    fn pump(&mut self, _state: &SyncplayGuiShellAppState) {}
}

#[allow(dead_code)]
#[derive(Default)]
struct GuiPreviewRuntimeOwner;

#[allow(dead_code)]
impl GuiPreviewRuntimeOwner {
    fn push_preview_response(handle: &GuiQueuedRuntimeBridgeHandle, request: GuiRuntimeRequest) {
        let actions = request.preview_actions();
        if !actions.is_empty() {
            handle.push_actions(actions);
        }
    }
}

impl GuiQueuedRuntimeOwner for GuiPreviewRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, _state: &SyncplayGuiShellAppState) {
        for request in handle.drain_requests() {
            Self::push_preview_response(handle, request);
        }
    }
}

struct GuiNoopClientRuntimePlayer;

impl PlayerAdapter for GuiNoopClientRuntimePlayer {
    fn name(&self) -> &'static str {
        "gui-client-runtime-noop"
    }

    fn set_paused(&mut self, _paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
        Ok(())
    }

    fn set_position(
        &mut self,
        _position_seconds: f64,
    ) -> Result<(), syncplay_player_api::PlayerError> {
        Ok(())
    }

    fn set_playback_rate(&mut self, _rate: f64) -> Result<(), syncplay_player_api::PlayerError> {
        Ok(())
    }
}

#[derive(Default)]
struct GuiTestPlayerAdapter {
    local_file_updates: VecDeque<LocalFileUpdate>,
    playback_updates: VecDeque<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
}

impl GuiTestPlayerAdapter {
    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        LocalFileUpdate::new(name).with_path(path.to_owned())
    }
}

impl PlayerAdapter for GuiTestPlayerAdapter {
    fn name(&self) -> &'static str {
        "test"
    }

    fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
        self.local_file_updates
            .push_back(Self::local_file_update_for_path(path));
        self.playback_updates.push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(0.0),
        );
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
        self.playback_updates.push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(paused),
        );
        Ok(())
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), syncplay_player_api::PlayerError> {
        self.playback_updates.push_back(
            syncplay_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(position_seconds),
        );
        Ok(())
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.local_file_updates.pop_front()
    }

    fn take_playback_telemetry_update(
        &mut self,
    ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
        self.playback_updates.pop_front()
    }
}

enum GuiOwnedPlayer {
    Test(GuiTestPlayerAdapter),
    Mpv(Box<MpvAdapter>),
    Custom(Box<dyn PlayerAdapter>),
}

impl GuiOwnedPlayer {
    fn name(&self) -> &'static str {
        match self {
            Self::Test(player) => player.name(),
            Self::Mpv(player) => player.name(),
            Self::Custom(player) => player.name(),
        }
    }

    fn as_mpv_mut(&mut self) -> Option<&mut MpvAdapter> {
        match self {
            Self::Mpv(player) => Some(player),
            Self::Test(_) | Self::Custom(_) => None,
        }
    }
}

impl PlayerAdapter for GuiOwnedPlayer {
    fn name(&self) -> &'static str {
        self.name()
    }

    fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.open_file(path),
            Self::Mpv(player) => player.open_file(path),
            Self::Custom(player) => player.open_file(path),
        }
    }

    fn set_option_string(
        &mut self,
        name: &str,
        value: &str,
    ) -> Result<(), syncplay_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_option_string(name, value),
            Self::Mpv(player) => player.set_option_string(name, value),
            Self::Custom(player) => player.set_option_string(name, value),
        }
    }

    fn apply_profile(&mut self, profile: &str) -> Result<(), syncplay_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.apply_profile(profile),
            Self::Mpv(player) => player.apply_profile(profile),
            Self::Custom(player) => player.apply_profile(profile),
        }
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_paused(paused),
            Self::Mpv(player) => player.set_paused(paused),
            Self::Custom(player) => player.set_paused(paused),
        }
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), syncplay_player_api::PlayerError> {
        match self {
            Self::Test(player) => player.set_position(position_seconds),
            Self::Mpv(player) => player.set_position(position_seconds),
            Self::Custom(player) => player.set_position(position_seconds),
        }
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        match self {
            Self::Test(player) => player.take_local_file_update(),
            Self::Mpv(player) => player.take_local_file_update(),
            Self::Custom(player) => player.take_local_file_update(),
        }
    }

    fn take_playback_telemetry_update(
        &mut self,
    ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
        match self {
            Self::Test(player) => player.take_playback_telemetry_update(),
            Self::Mpv(player) => player.take_playback_telemetry_update(),
            Self::Custom(player) => player.take_playback_telemetry_update(),
        }
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        match self {
            Self::Test(player) => player.take_pending_chat_request(),
            Self::Mpv(player) => player.take_pending_chat_request(),
            Self::Custom(player) => player.take_pending_chat_request(),
        }
    }
}

trait GuiSessionRuntimeAdapter {
    fn drain_gui_actions(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn playlist_control_available(&self) -> bool {
        false
    }

    fn adjust_command_availability(
        &self,
        _state: &SyncplayGuiShellAppState,
        command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        command_availability
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn apply_message_json(&mut self, _json_line: &str) -> Result<(), String> {
        Err(
            "Attached session runtime does not accept inbound protocol transport messages."
                .to_owned(),
        )
    }

    fn set_room(&mut self, _room: String) -> Result<(), String> {
        Err("Attached session runtime does not support room changes.".to_owned())
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        self.set_room(default_room)
    }

    fn set_local_ready(&mut self, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support local readiness changes.".to_owned())
    }

    fn set_user_ready(&mut self, _username: String, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support remote readiness changes.".to_owned())
    }

    fn request_controller_auth(&mut self, _room: String, _password: String) -> Result<(), String> {
        Err("Attached session runtime does not support controller auth requests.".to_owned())
    }

    fn queue_playlist_entry(
        &mut self,
        _entry: String,
        _select_after_queue: bool,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist queue operations."
                .to_owned(),
        )
    }

    fn set_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist selection changes."
                .to_owned(),
        )
    }

    fn delete_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist removal.".to_owned())
    }

    fn replace_playlist(
        &mut self,
        _files: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist reorder operations."
                .to_owned(),
        )
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist undo.".to_owned())
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn sync_local_playback_telemetry(
        &mut self,
        _paused: Option<bool>,
        _position_seconds: Option<f64>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn set_playback_paused(&mut self, _paused: bool) -> Result<bool, String> {
        Err("Attached session runtime does not support playback pause changes.".to_owned())
    }

    fn record_manual_seek_to_position(&mut self, _position_seconds: f64) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek history.".to_owned())
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek undo.".to_owned())
    }

    fn local_position_seconds(&self) -> Option<f64> {
        None
    }

    fn set_autoplay_enabled(&mut self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, _threshold: usize) -> Result<(), String> {
        Ok(())
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String>;

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String>;

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
    ) -> Result<Vec<(String, String)>, String>;

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String>;
}

#[allow(dead_code)]
struct GuiClientCoreChatSessionRuntimeAdapter {
    username: String,
    baseline_room: String,
    runtime: ClientRuntime<GuiNoopClientRuntimePlayer, QueuedRuntimeControl>,
    pending_startup_protocol_lines: VecDeque<String>,
    next_state_sync_heartbeat_at: Option<Instant>,
    next_autoplay_tick_at: Option<Instant>,
    tracked_remote_usernames: BTreeSet<String>,
}

#[allow(dead_code)]
impl GuiClientCoreChatSessionRuntimeAdapter {
    const STATE_SYNC_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

    fn new(username: impl Into<String>, room: impl Into<String>) -> Result<Self, String> {
        Self::new_with_control_password(username, room, None)
    }

    fn new_with_control_password(
        username: impl Into<String>,
        room: impl Into<String>,
        controlled_room_password_override: Option<String>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let hello_json = Self::hello_json(&username, &room);
        let mut session = ClientSession::default();
        if let Some(control_password) = controlled_room_password_override.as_deref() {
            session.remember_control_password_for_room(&room, control_password);
        }

        Ok(Self {
            username,
            baseline_room: room,
            runtime: ClientRuntime::new(
                session,
                GuiNoopClientRuntimePlayer,
                QueuedRuntimeControl::default(),
            ),
            pending_startup_protocol_lines: VecDeque::from([hello_json]),
            next_state_sync_heartbeat_at: None,
            next_autoplay_tick_at: None,
            tracked_remote_usernames: BTreeSet::new(),
        })
    }

    fn hello_json(username: &str, room: &str) -> String {
        format!(
            r#"{{"Hello":{{"username":{username:?},"room":{{"name":{room:?}}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
        )
    }

    fn current_room_for_next_hello(&self) -> String {
        self.runtime
            .session()
            .local_room_command_target_with_legacy_fallback(&self.baseline_room)
    }

    fn reset_session_for_reconnect(&mut self) {
        let room = self.current_room_for_next_hello();
        self.baseline_room = room.clone();
        self.runtime = ClientRuntime::new(
            ClientSession::default(),
            GuiNoopClientRuntimePlayer,
            QueuedRuntimeControl::default(),
        );
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(&self.username, &room));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.tracked_remote_usernames.clear();
    }

    fn queue_periodic_state_sync_heartbeat_if_due(&mut self) {
        if self.runtime.session().server_chat_supported().is_none() {
            self.next_state_sync_heartbeat_at = None;
            return;
        }

        let now = Instant::now();
        let Some(next_heartbeat_at) = self.next_state_sync_heartbeat_at else {
            self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
            return;
        };
        if now < next_heartbeat_at {
            return;
        }

        let _ = self
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible();
        self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
    }

    fn autoplay_runtime_flags(&self) -> (bool, bool, bool, bool) {
        let session = self.runtime.session();
        let readiness_supported = session.server_readiness_supported().unwrap_or(false);
        let local_can_control = session.local_can_control().unwrap_or(false);
        let is_playing_music = session.is_playing_music();
        let recently_advanced = session.recently_advanced(system_time_seconds());
        (
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        )
    }

    fn sync_autoplay_runtime(&mut self, actions: &mut Vec<GuiShellAction>) {
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );

        if !self.runtime.session().autoplay_timer_is_running() {
            self.next_autoplay_tick_at = None;
            return;
        }

        let tick_interval =
            Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS.max(f64::EPSILON));
        let now = Instant::now();
        let Some(next_autoplay_tick_at) = self.next_autoplay_tick_at else {
            self.next_autoplay_tick_at = Some(now + tick_interval);
            return;
        };
        if now < next_autoplay_tick_at {
            return;
        }

        if let Err(error) = self.runtime.tick_autoplay(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core autoplay dispatch failed: {error}"),
            });
            self.next_autoplay_tick_at = None;
            return;
        }

        self.next_autoplay_tick_at = if self.runtime.session().autoplay_timer_is_running() {
            Some(now + tick_interval)
        } else {
            None
        };
    }

    fn session_media_search_target(&self) -> Option<String> {
        if let Some(file_name) =
            self.runtime
                .session()
                .current_room_playlist()
                .and_then(|playlist| {
                    playlist
                        .index
                        .and_then(|index| usize::try_from(index).ok())
                        .and_then(|index| playlist.files.get(index))
                })
        {
            let trimmed = file_name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }

        if let Some(file_name) = self
            .runtime
            .session()
            .username
            .as_deref()
            .and_then(|username| self.runtime.session().user_file_name(username))
        {
            let trimmed = file_name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }

        self.tracked_remote_usernames.iter().find_map(|username| {
            self.runtime
                .session()
                .user_file_name(username)
                .map(str::trim)
                .filter(|file_name| !file_name.is_empty())
                .map(str::to_owned)
        })
    }

    fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        let Some(target) = self.session_media_search_target() else {
            return Err(
                "Client-core session runtime cannot search missing media because the current session does not expose a target file."
                    .to_owned(),
            );
        };
        if target.contains("://") {
            return Err(
                "Client-core session runtime cannot search missing media for URL-based media targets."
                    .to_owned(),
            );
        }
        let Some(file_name) = Path::new(&target)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return Err(
                "Client-core session runtime could not derive a file name for missing-media search."
                    .to_owned(),
            );
        };
        let file_name = file_name.trim();
        if file_name.is_empty() {
            return Err(
                "Client-core session runtime could not derive a non-empty file name for missing-media search."
                    .to_owned(),
            );
        }
        Ok(file_name.to_owned())
    }

    fn missing_media_file_name_matches(target: &str, candidate: &str) -> bool {
        if cfg!(windows) {
            candidate.eq_ignore_ascii_case(target)
        } else {
            candidate == target
        }
    }

    fn search_path_for_missing_media_target(
        target_file_name: &str,
        path: &Path,
    ) -> Result<Option<String>, String> {
        if path.is_file() {
            let matches_target =
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|candidate| {
                        Self::missing_media_file_name_matches(target_file_name, candidate)
                    });
            if matches_target {
                return Ok(Some(path.to_string_lossy().into_owned()));
            }
            return Ok(None);
        }

        if !path.is_dir() {
            return Ok(None);
        }

        let mut children = std::fs::read_dir(path)
            .map_err(|error| {
                format!(
                    "Client-core session runtime could not scan '{}' during missing-media search: {error}",
                    path.display()
                )
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect::<Vec<_>>();
        children.sort();

        for child in children {
            if let Some(found_path) =
                Self::search_path_for_missing_media_target(target_file_name, &child)?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    fn normalize_public_server_rows(
        current_servers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let mut normalized = Vec::new();
        let mut seen_addresses = BTreeSet::new();
        for (label, address) in current_servers {
            let Some(label) = normalized_editable_text(&label) else {
                continue;
            };
            let Some(address) = normalized_editable_text(&address) else {
                continue;
            };
            let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
            if host.trim().is_empty() {
                continue;
            }
            let dedupe_key = address.to_ascii_lowercase();
            if !seen_addresses.insert(dedupe_key) {
                continue;
            }
            normalized.push((label, address));
        }
        normalized
    }

    fn refreshed_public_server_rows_from_lookup<F>(
        lookup: &F,
    ) -> Result<Option<Vec<(String, String)>>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_name = "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS";
        let Some(value) = lookup(env_name) else {
            return Ok(None);
        };
        let Some(parsed) = parse_serialized_public_servers_list_legacy_compatible(&value) else {
            return Err(format!(
                "{env_name} must be a serialized public-server list like [[\"Primary\", \"syncplay.pl:8999\"]]."
            ));
        };
        Ok(Some(Self::normalize_public_server_rows(parsed)))
    }

    fn refreshed_public_server_rows_from_sources<F, R>(
        lookup: &F,
        read_to_string: &R,
    ) -> Result<Option<Vec<(String, String)>>, String>
    where
        F: Fn(&str) -> Option<String>,
        R: Fn(&str) -> Result<String, String>,
    {
        let path_env_name = "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH";
        if let Some(path) = lookup(path_env_name) {
            let value = read_to_string(&path)
                .map_err(|error| format!("{path_env_name} could not read '{path}': {error}"))?;
            let Some(parsed) = parse_serialized_public_servers_list_legacy_compatible(&value)
            else {
                return Err(format!(
                    "{path_env_name} file '{path}' must be a serialized public-server list like [[\"Primary\", \"syncplay.pl:8999\"]]."
                ));
            };
            return Ok(Some(Self::normalize_public_server_rows(parsed)));
        }

        Self::refreshed_public_server_rows_from_lookup(lookup)
    }

    fn refreshed_public_server_rows_from_env() -> Result<Option<Vec<(String, String)>>, String> {
        Self::refreshed_public_server_rows_from_sources(&env_trimmed, &|path| {
            std::fs::read_to_string(path).map_err(|error| error.to_string())
        })
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        let mut lines: Vec<_> = self.pending_startup_protocol_lines.drain(..).collect();
        lines.extend(
            self.runtime
                .flush_queued_protocol_lines()
                .map_err(|error| format!("Queued protocol line encoding failed: {error}"))?,
        );
        Ok(lines)
    }

    fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        self.runtime
            .session_mut()
            .apply_message_json(json_line)
            .map_err(|error| format!("Inbound client-session message apply failed: {error}"))
    }

    fn note_user_change(&mut self, notification: UserChangeNotification) {
        match notification {
            UserChangeNotification::Joined { username, .. }
            | UserChangeNotification::Playing { username, .. } => {
                self.tracked_remote_usernames.insert(username);
            }
            UserChangeNotification::Left { username, .. } => {
                self.tracked_remote_usernames.remove(&username);
            }
        }
    }

    fn user_change_action(notification: UserChangeNotification) -> Option<GuiShellAction> {
        let message = match notification {
            UserChangeNotification::Joined {
                username,
                room,
                hide_from_osd,
            } => (!hide_from_osd).then(|| format!("{username} joined {room}.")),
            UserChangeNotification::Playing {
                username,
                room,
                file_name,
                include_room_addendum,
                hide_from_osd,
                ..
            } => {
                if hide_from_osd {
                    None
                } else {
                    let media_label = file_name.unwrap_or_else(|| "media".to_owned());
                    let room_addendum = if include_room_addendum {
                        format!(" in {room}")
                    } else {
                        String::new()
                    };
                    Some(format!(
                        "{username} is playing {media_label}{room_addendum}."
                    ))
                }
            }
            UserChangeNotification::Left {
                username,
                hide_from_osd,
            } => (!hide_from_osd).then(|| format!("{username} left.")),
        }?;
        Some(GuiShellAction::AnnounceSystemChatEvent(message))
    }

    fn reconnect_transition_actions(
        notification: ReconnectTransitionNotification,
    ) -> Vec<GuiShellAction> {
        let (level, message, persist_to_system_chat) = match notification {
            ReconnectTransitionNotification::Attempting {
                retries,
                delay_seconds,
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Reconnect attempt {} in {:.1} seconds.",
                    retries.saturating_add(1),
                    delay_seconds
                ),
                true,
            ),
            ReconnectTransitionNotification::Connected => (
                GuiTransientNotificationLevel::Success,
                "Session reconnected.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::Disconnected => (
                GuiTransientNotificationLevel::Warning,
                "Session disconnected.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::RestoringState => (
                GuiTransientNotificationLevel::Info,
                "Restoring session state.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                position_diff_seconds,
                ..
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Session state restore mismatch detected ({position_diff_seconds:.3} seconds)."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt,
                max_attempts,
                cooldown_ticks,
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Session state correction retry {attempt}/{max_attempts} scheduled after {cooldown_ticks} ticks."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts,
                max_attempts,
            } => (
                GuiTransientNotificationLevel::Error,
                format!(
                    "Session state correction exhausted after {attempts}/{max_attempts} attempts."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles,
                disable_after_mismatch_cycles,
            } => (
                GuiTransientNotificationLevel::Error,
                format!(
                    "Session state correction disabled after {consecutive_mismatch_cycles}/{disable_after_mismatch_cycles} mismatch cycles."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                remaining_reconnect_cycles_after_this_cycle,
            } => (
                GuiTransientNotificationLevel::Info,
                format!(
                    "Session state correction recovery cooldown active for {remaining_reconnect_cycles_after_this_cycle} more reconnect cycles."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled => (
                GuiTransientNotificationLevel::Info,
                "Session state correction recovery cooldown ended.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::RestoringPlaylist => (
                GuiTransientNotificationLevel::Info,
                "Restoring shared playlist state.".to_owned(),
                true,
            ),
        };
        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level,
            message: message.clone(),
        }];
        if persist_to_system_chat {
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        actions
    }

    fn controller_auth_transition_action(
        notification: ControllerAuthTransitionNotification,
    ) -> Vec<GuiShellAction> {
        let (level, message) = match notification {
            ControllerAuthTransitionNotification::Attempting { room } => (
                GuiTransientNotificationLevel::Info,
                format!("Requesting controller access for {room}."),
            ),
            ControllerAuthTransitionNotification::Succeeded {
                username,
                room,
                hide_from_osd,
            } => {
                if hide_from_osd {
                    return Vec::new();
                }
                (
                    GuiTransientNotificationLevel::Success,
                    format!("{username} received controller access for {room}."),
                )
            }
            ControllerAuthTransitionNotification::Failed {
                username,
                room,
                hide_from_osd,
            } => {
                if hide_from_osd {
                    return Vec::new();
                }
                (
                    GuiTransientNotificationLevel::Error,
                    format!("Controller access failed for {username} in {room}."),
                )
            }
        };
        vec![
            GuiShellAction::PushTransientNotification {
                level,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn controlled_room_creation_action(
        notification: ControlledRoomCreationNotification,
    ) -> Vec<GuiShellAction> {
        match notification {
            ControlledRoomCreationNotification::Created { room, password } => {
                let share_code = format!("{room}:{password}");
                let transient_message = format!("Controlled room created: {room}.");
                let chat_message = format!(
                    "Created controlled room {room} with password {password} ({share_code})."
                );
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Success,
                        message: transient_message,
                    },
                    GuiShellAction::AnnounceSystemChatEvent(chat_message),
                ]
            }
        }
    }

    fn autoplay_countdown_action(
        notification: AutoplayCountdownNotification,
    ) -> Vec<GuiShellAction> {
        let message = format!(
            "Autoplay in {} seconds with {} ready users.",
            notification.seconds_left, notification.ready_user_count
        );
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn shared_playlist_control_available(&self) -> bool {
        self.runtime.session().local_can_control().unwrap_or(false)
    }

    fn session_runtime_rooms(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<MainWindowRuntimeRoomSnapshot> {
        let session = self.runtime.session();
        let mut rooms = session
            .room_names()
            .into_iter()
            .filter_map(|room_name| {
                normalized_editable_text(&room_name).map(|room_name| {
                    MainWindowRuntimeRoomSnapshot {
                        has_named_users: !session.usernames_in_room(&room_name).is_empty(),
                        is_controlled: room_name.starts_with('+'),
                        room_name,
                    }
                })
            })
            .collect::<Vec<_>>();
        if rooms.is_empty()
            && let Some(room_name) = normalized_editable_text(&state.main_window.room_name)
        {
            rooms.push(MainWindowRuntimeRoomSnapshot {
                has_named_users: false,
                is_controlled: room_name.starts_with('+'),
                room_name,
            });
        }
        rooms
    }

    fn session_runtime_users(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<MainWindowRuntimeUserSnapshot> {
        let session = self.runtime.session();
        let settings = state.configuration.to_stored_settings();
        let trusted_domains = settings.trusted_domains.unwrap_or_default();
        let only_switch_to_trusted_domains =
            settings.only_switch_to_trusted_domains.unwrap_or(true);
        let local_username = session.username.as_deref();
        let mut users = Vec::new();
        for room_name in session.room_names() {
            for username in session.usernames_in_room(&room_name) {
                let is_self = local_username == Some(username.as_str());
                let file_name = session
                    .user_file_name(&username)
                    .and_then(normalized_editable_text);
                let file_is_url = file_name.as_deref().is_some_and(browser_is_url);
                let file_is_trusted = file_name.as_deref().is_none_or(|file_name| {
                    browser_uri_is_trusted(
                        file_name,
                        only_switch_to_trusted_domains,
                        &trusted_domains,
                    )
                });
                let differences = session
                    .file_differences_for_user(&username)
                    .unwrap_or_default();
                users.push(MainWindowRuntimeUserSnapshot {
                    username: username.clone(),
                    room_name: room_name.clone(),
                    is_self,
                    is_ready: session.user_ready(&username).unwrap_or(false),
                    is_controller: session.user_controller(&username).unwrap_or(false),
                    has_file: session
                        .user_has_file(&username)
                        .unwrap_or(file_name.is_some()),
                    file_name,
                    file_size_label: browser_format_size_label(session.user_file_size(&username)),
                    file_duration_label: browser_format_duration_label(
                        session.user_file_duration(&username),
                    ),
                    file_is_url,
                    file_is_trusted,
                    filename_differs: differences.filename,
                    filesize_differs: differences.filesize,
                    fileduration_differs: differences.fileduration,
                });
            }
        }
        users
    }

    fn main_window_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<MainWindowRuntimeSnapshot> {
        let baseline_main_window =
            MainWindowShellState::from_stored_settings(&state.configuration.to_stored_settings());
        let session = self.runtime.session();
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        snapshot.room_name = baseline_main_window.room_name.clone();
        snapshot.shared_playlist_enabled = baseline_main_window.shared_playlist_enabled;
        snapshot.controlled_room_active = baseline_main_window.controlled_room_active;
        snapshot.hide_empty_rooms = state.main_window.hide_empty_rooms;
        snapshot.rooms = baseline_main_window
            .rooms
            .clone()
            .into_iter()
            .map(|room| MainWindowRuntimeRoomSnapshot {
                room_name: room.room_name,
                is_controlled: room.is_controlled,
                has_named_users: room.has_named_users,
            })
            .collect();
        snapshot.users = baseline_main_window
            .users
            .iter()
            .map(|user| MainWindowRuntimeUserSnapshot {
                username: user.username.clone(),
                room_name: user.room_name.clone(),
                is_self: user.is_self,
                is_ready: user.is_ready,
                is_controller: user.is_controller,
                has_file: user.has_file,
                file_name: user.file_name.clone(),
                file_size_label: user.file_size_label.clone(),
                file_duration_label: user.file_duration_label.clone(),
                file_is_url: user.file_is_url,
                file_is_trusted: user.file_is_trusted,
                filename_differs: user.filename_differs,
                filesize_differs: user.filesize_differs,
                fileduration_differs: user.fileduration_differs,
            })
            .collect();
        snapshot.playlist = baseline_main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect();
        snapshot.can_set_ready = baseline_main_window.playback.can_set_ready;
        snapshot.can_set_others_ready = baseline_main_window.playback.can_set_others_ready;
        snapshot.playback_paused = baseline_main_window.playback_paused;
        snapshot.autoplay_active = state.main_window.autoplay_active;
        snapshot.autoplay_threshold = state.main_window.autoplay_threshold;
        snapshot.autoplay_countdown_seconds = state.main_window.autoplay_countdown_seconds;
        snapshot.user_offset_seconds = state.main_window.user_offset_seconds;
        snapshot.show_playback_buttons = state.main_window.show_playback_buttons;
        snapshot.show_autoplay_controls = state.main_window.show_autoplay_controls;
        if let Some(room_name) = session
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let controlled_room_active = room_name.starts_with('+');
            snapshot.room_name = room_name.to_owned();
            snapshot.controlled_room_active = controlled_room_active;
            snapshot.rooms = self.session_runtime_rooms(state);
            snapshot.users = self.session_runtime_users(state);
        }
        if let Some(playlist) = session.current_room_playlist() {
            snapshot.shared_playlist_enabled = true;
            snapshot.playlist = playlist.files.clone();
        }
        snapshot.can_manage_playlist =
            snapshot.shared_playlist_enabled && self.shared_playlist_control_available();
        snapshot.can_undo_seek = session.last_seek_position_before_manual_seek().is_some();
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.autoplay_active = session.autoplay_enabled();
        snapshot.autoplay_threshold = session
            .readiness_autoplay_config()
            .auto_play_threshold
            .unwrap_or(DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD);
        snapshot.autoplay_countdown_seconds = session
            .autoplay_timer_is_running()
            .then(|| session.autoplay_time_left_seconds().max(0.0).floor() as u32);
        if let Some(playstate) = session.current_room_playstate()
            && let Some(paused) = playstate.paused
        {
            snapshot.playback_paused = paused;
        }
        if let Some(paused) = session.local_paused() {
            snapshot.playback_paused = paused;
        }
        if session.server_chat_supported().is_none() {
            snapshot.can_set_ready = false;
        } else if let Some(server_readiness_supported) = session.server_readiness_supported() {
            snapshot.can_set_ready = server_readiness_supported;
        }
        snapshot.can_set_others_ready = session
            .server_set_others_readiness_supported()
            .unwrap_or(false);
        (snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window))
            .then_some(snapshot)
    }

    fn session_playlist_selection_index(&self, playlist_len: usize) -> Option<usize> {
        self.runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|&index| index < playlist_len)
    }

    fn interaction_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
        playlist_len: usize,
    ) -> Option<GuiInteractionRuntimeSnapshot> {
        let selected_main_window_playlist = self.session_playlist_selection_index(playlist_len);
        if state.selection.selected_main_window_playlist == selected_main_window_playlist {
            return None;
        }

        let mut snapshot = GuiInteractionRuntimeSnapshot::from_shell_state(state);
        snapshot.selection.selected_main_window_playlist = selected_main_window_playlist;
        Some(snapshot)
    }

    fn menu_dialog_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
        shared_playlist_enabled: bool,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let mut action_overrides = Vec::new();
        let settings = state.configuration.to_stored_settings();
        let session_room_name = self
            .runtime
            .session()
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let create_controlled_room_enabled = session_room_name.is_some();
        let identify_as_controller_enabled =
            session_room_name.is_some_and(|room_name| room_name.starts_with('+'));
        let config_chat_enabled = settings.chat_input_enabled.unwrap_or(false)
            || settings.chat_output_enabled.unwrap_or(false);
        let desired_show_chat_enabled =
            config_chat_enabled && self.runtime.session().server_chat_supported() == Some(true);

        let current_show_chat_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .map(|action| action.enabled);
        if current_show_chat_enabled
            .is_some_and(|current_enabled| current_enabled != desired_show_chat_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: desired_show_chat_enabled,
            });
        }

        let current_show_playlist_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .map(|action| action.enabled);
        if current_show_playlist_enabled
            .is_some_and(|current_enabled| current_enabled != shared_playlist_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: shared_playlist_enabled,
            });
        }

        let current_playlist_actions_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Playlist Actions")
            })
            .map(|action| action.enabled);
        let desired_playlist_actions_enabled =
            shared_playlist_enabled && self.shared_playlist_control_available();
        if current_playlist_actions_enabled
            .is_some_and(|current_enabled| current_enabled != desired_playlist_actions_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: desired_playlist_actions_enabled,
            });
        }

        for (action_label, enabled) in [
            ("Create Controlled Room", create_controlled_room_enabled),
            ("Identify As Controller", identify_as_controller_enabled),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == "Advanced")
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label,
                    enabled,
                });
            }
        }

        if action_overrides.is_empty() {
            return None;
        }

        Some(MenuDialogRuntimeSnapshot {
            action_overrides,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        })
    }
}

impl GuiSessionRuntimeAdapter for GuiClientCoreChatSessionRuntimeAdapter {
    fn drain_gui_actions(&mut self, state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        let mut trailing_actions = Vec::new();
        if let Err(error) = self.runtime.run_user_change_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core user-change dispatch failed: {error}"),
            });
        } else {
            for notification in self.runtime.drain_user_change_notifications() {
                self.note_user_change(notification.clone());
                if let Some(action) = Self::user_change_action(notification) {
                    trailing_actions.push(action);
                }
            }
        }
        if let Err(error) = self.runtime.run_reconnect_transition_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect transition dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(Self::reconnect_transition_actions),
            );
        }
        if let Err(error) = self.runtime.run_reconnect_state_restore_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect state-restore dispatch failed: {error}"),
            });
        }
        if let Err(error) = self
            .runtime
            .run_reconnect_state_restore_validation_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect validation dispatch failed: {error}"),
            });
        }
        if !actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    ..
                }
            )
        }) {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(Self::reconnect_transition_actions),
            );
        } else {
            self.runtime.drain_reconnect_notifications();
        }
        if let Err(error) = self
            .runtime
            .run_controlled_room_creation_notifications_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controlled-room dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controlled_room_creation_notifications()
                    .into_iter()
                    .flat_map(Self::controlled_room_creation_action),
            );
        }
        if let Err(error) = self.runtime.run_controller_reidentify_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller reidentify dispatch failed: {error}"),
            });
        }
        if let Err(error) = self.runtime.run_controller_auth_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller-auth dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controller_auth_notifications()
                    .into_iter()
                    .flat_map(Self::controller_auth_transition_action),
            );
        }
        self.sync_autoplay_runtime(&mut actions);
        trailing_actions.extend(
            self.runtime
                .drain_autoplay_notifications()
                .into_iter()
                .flat_map(Self::autoplay_countdown_action),
        );
        if let Err(error) = self.runtime.run_chat_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core chat notification dispatch failed: {error}"),
            });
        }
        self.queue_periodic_state_sync_heartbeat_if_due();

        let main_window_runtime_snapshot = self.main_window_runtime_snapshot(state);
        let interaction_runtime_snapshot = self.interaction_runtime_snapshot(
            state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.playlist.len())
                .unwrap_or_else(|| state.main_window.playlist.len()),
        );
        let menu_dialog_runtime_snapshot = self.menu_dialog_runtime_snapshot(
            state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.shared_playlist_enabled)
                .unwrap_or(state.main_window.shared_playlist_enabled),
        );
        if let Some(snapshot) = main_window_runtime_snapshot
            && snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window)
        {
            actions.push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = interaction_runtime_snapshot {
            actions.push(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = menu_dialog_runtime_snapshot {
            actions.push(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot));
        }

        actions.extend(
            self.runtime
                .drain_chat_notifications()
                .into_iter()
                .map(|notification| match notification {
                    ChatNotification::Message { username, message } => {
                        GuiShellAction::PushChatMessage {
                            sender: username.unwrap_or_else(|| "Server".to_owned()),
                            message,
                        }
                    }
                }),
        );
        actions.extend(trailing_actions);
        actions
    }

    fn adjust_command_availability(
        &self,
        _state: &SyncplayGuiShellAppState,
        mut command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        if self.runtime.session().server_chat_supported() != Some(true) {
            command_availability.can_send_chat_message = false;
        }
        command_availability
    }

    fn playlist_control_available(&self) -> bool {
        self.shared_playlist_control_available()
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        GuiClientCoreChatSessionRuntimeAdapter::flush_outbound_protocol_lines(self)
    }

    fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        GuiClientCoreChatSessionRuntimeAdapter::apply_message_json(self, json_line)
    }

    fn set_room(&mut self, room: String) -> Result<(), String> {
        match self.runtime.run_set_room(room) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        match self.runtime.run_set_room_with_legacy_fallback(default_room) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String> {
        match self.runtime.run_send_chat_message(message) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_chat_supported() {
                None => Err(
                    "Client-core session runtime cannot send chat until the server Hello enables chat."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot send chat because the server disabled chat."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound chat message."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime chat dispatch failed: {error}"
            )),
        }
    }

    fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user("", ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change readiness until the server Hello enables readiness."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change readiness because the server disabled readiness."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound readiness change."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn set_user_ready(&mut self, username: String, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user(username, ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_set_others_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change other users' readiness until the server Hello enables remote readiness changes."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change other users' readiness because the server disabled remote readiness changes."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound remote readiness change."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn request_controller_auth(&mut self, room: String, password: String) -> Result<(), String> {
        match self.runtime.run_request_controller_auth(room, password) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().username.is_none() {
                    Err(
                        "Client-core session runtime cannot request controller access until the server Hello is received."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound controller-auth request."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime controller-auth dispatch failed: {error}"
            )),
        }
    }

    fn queue_playlist_entry(
        &mut self,
        entry: String,
        select_after_queue: bool,
    ) -> Result<(), String> {
        match self
            .runtime
            .run_queue_playlist_item(entry, select_after_queue)
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist entry."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist queue dispatch failed: {error}"
            )),
        }
    }

    fn set_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_set_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist selection before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist selection change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist selection dispatch failed: {error}"
            )),
        }
    }

    fn delete_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_delete_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot remove shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist removal."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist removal dispatch failed: {error}"
            )),
        }
    }

    fn replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<(), String> {
        match self.runtime.run_replace_playlist(files, selected_index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot reorder the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist reorder."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist reorder dispatch failed: {error}"
            )),
        }
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        match self.runtime.run_undo_playlist_change() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot undo shared playlist changes before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist undo."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist undo dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_remaining_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle remaining shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_entire_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn sync_local_playback_telemetry(
        &mut self,
        paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .apply_player_playback_telemetry_update(&PlayerPlaybackTelemetryUpdate {
                paused,
                position_seconds,
                playback_rate: None,
            });
        Ok(())
    }

    fn set_playback_paused(&mut self, paused: bool) -> Result<bool, String> {
        match self.runtime.run_set_paused(paused) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime playback pause dispatch failed: {error}"
            )),
        }
    }

    fn record_manual_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, String> {
        match self.runtime.run_seek_to_position(position_seconds) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime seek dispatch failed: {error}"
            )),
        }
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        match self.runtime.run_undo_seek() {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime undo-seek dispatch failed: {error}"
            )),
        }
    }

    fn local_position_seconds(&self) -> Option<f64> {
        self.runtime.session().local_position_seconds()
    }

    fn set_autoplay_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.runtime.session_mut().set_autoplay_enabled(enabled);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, threshold: usize) -> Result<(), String> {
        self.runtime
            .session_mut()
            .readiness_autoplay_config_mut()
            .auto_play_threshold = Some(threshold);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        let Some((_label, address)) = selected_server else {
            return Err(
                "Client-core session runtime cannot connect because no public server is selected."
                    .to_owned(),
            );
        };
        let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
        if host.trim().is_empty() {
            return Err(
                "Client-core session runtime cannot connect because the selected public-server address is invalid."
                    .to_owned(),
            );
        }
        self.reset_session_for_reconnect();
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        _current_servers: Vec<(String, String)>,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(refreshed_servers) = Self::refreshed_public_server_rows_from_env()? {
            return Ok(refreshed_servers);
        }
        #[cfg(test)]
        {
            Ok(Self::normalize_public_server_rows(_current_servers))
        }
        #[cfg(not(test))]
        {
            let refreshed_servers = remote_services::fetch_public_servers(Some("en"))?;
            Ok(Self::normalize_public_server_rows(refreshed_servers))
        }
    }

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String> {
        let target_file_name = self.missing_media_search_target_file_name()?;
        for directory in directories {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                Self::search_path_for_missing_media_target(&target_file_name, Path::new(trimmed))?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }
}

#[allow(dead_code)]
#[derive(Clone, Default)]
struct GuiQueuedSessionTransportHandle {
    queued_inbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
    queued_outbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
}

#[allow(dead_code)]
impl GuiQueuedSessionTransportHandle {
    fn push_inbound_protocol_line(&self, line: impl Into<String>) {
        self.push_inbound_protocol_lines([line.into()]);
    }

    fn push_inbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(lines);
    }

    fn drain_inbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    fn push_outbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(lines);
    }

    fn drain_outbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    fn clear_protocol_lines(&self) {
        self.queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

trait GuiSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String>;
}

#[allow(dead_code)]
struct GuiLoopbackSessionTransportDriver {
    echo_username: String,
}

#[allow(dead_code)]
impl GuiLoopbackSessionTransportDriver {
    fn new(echo_username: impl Into<String>) -> Self {
        Self {
            echo_username: echo_username.into(),
        }
    }

    fn json_string_literal(input: &str) -> Option<&str> {
        let mut characters = input.char_indices();
        match characters.next() {
            Some((_, '"')) => {}
            _ => return None,
        }

        let mut escaped = false;
        for (index, character) in characters {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => return Some(&input[..=index]),
                _ => {}
            }
        }
        None
    }

    fn chat_message_literal(line: &str) -> Option<&str> {
        let rest = line.strip_prefix("{\"Chat\":")?.strip_suffix('}')?;
        if rest.starts_with('"') {
            return Self::json_string_literal(rest);
        }

        let message_key = "\"message\":";
        let message_index = rest.find(message_key)?;
        let message_start = message_index + message_key.len();
        Self::json_string_literal(rest.get(message_start..)?)
    }

    fn translated_inbound_line(&self, outbound_line: &str) -> String {
        let Some(message_literal) = Self::chat_message_literal(outbound_line) else {
            return outbound_line.to_owned();
        };
        format!(
            r#"{{"Chat":{{"username":{:?},"message":{message_literal}}}}}"#,
            self.echo_username
        )
    }
}

impl GuiSessionTransportDriver for GuiLoopbackSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        let outbound_protocol_lines = transport.drain_outbound_protocol_lines();
        if outbound_protocol_lines.is_empty() {
            return Ok(());
        }
        transport.push_inbound_protocol_lines(
            outbound_protocol_lines
                .into_iter()
                .map(|line| self.translated_inbound_line(&line)),
        );
        Ok(())
    }
}

struct GuiTcpSessionTransportDriver {
    stream: Option<TcpStream>,
    pending_outbound_lines: VecDeque<Vec<u8>>,
    pending_outbound_offset: usize,
    inbound_buffer: Vec<u8>,
}

impl GuiTcpSessionTransportDriver {
    fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect((host, port)).map_err(|error| {
            format!("Session transport TCP connect to {host}:{port} failed: {error}")
        })?;
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("Session transport TCP nonblocking setup failed: {error}"))?;
        stream
            .set_nodelay(true)
            .map_err(|error| format!("Session transport TCP nodelay setup failed: {error}"))?;
        Ok(Self {
            stream: Some(stream),
            pending_outbound_lines: VecDeque::new(),
            pending_outbound_offset: 0,
            inbound_buffer: Vec::new(),
        })
    }

    fn connect_from_host_arg(host_arg: &str) -> Result<Self, String> {
        let (host, port) = parse_host_and_optional_port_from_host_arg_legacy_compatible(host_arg);
        let Some(port) = port.or(Some(8999)) else {
            return Err("Session transport TCP port resolution failed.".to_owned());
        };
        Self::connect(&host, port)
    }

    fn disconnect_with_error(&mut self, message: String) -> Result<(), String> {
        self.stream = None;
        self.pending_outbound_lines.clear();
        self.pending_outbound_offset = 0;
        self.inbound_buffer.clear();
        Err(message)
    }

    fn queue_outbound_lines(&mut self, transport: &GuiQueuedSessionTransportHandle) {
        for line in transport.drain_outbound_protocol_lines() {
            let mut encoded_line = line.into_bytes();
            encoded_line.extend_from_slice(b"\r\n");
            self.pending_outbound_lines.push_back(encoded_line);
        }
    }

    fn flush_outbound_lines(&mut self) -> Result<(), String> {
        while !self.pending_outbound_lines.is_empty() {
            let Some(stream) = self.stream.as_mut() else {
                return Ok(());
            };
            let Some(front) = self.pending_outbound_lines.front() else {
                break;
            };
            let front_len = front.len();
            let pending_slice = &front[self.pending_outbound_offset..];
            match stream.write(pending_slice) {
                Ok(0) => {
                    return self.disconnect_with_error(
                        "Session transport TCP connection closed while writing.".to_owned(),
                    );
                }
                Ok(written) => {
                    self.pending_outbound_offset += written;
                    if self.pending_outbound_offset >= front_len {
                        self.pending_outbound_lines.pop_front();
                        self.pending_outbound_offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return self.disconnect_with_error(format!(
                        "Session transport TCP write failed: {error}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn drain_inbound_lines(
        &mut self,
        transport: &GuiQueuedSessionTransportHandle,
    ) -> Result<(), String> {
        let mut read_buffer = [0_u8; 4096];
        let mut closed_by_server = false;
        loop {
            let Some(stream) = self.stream.as_mut() else {
                break;
            };
            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    self.stream = None;
                    closed_by_server = true;
                    break;
                }
                Ok(read_bytes) => self
                    .inbound_buffer
                    .extend_from_slice(&read_buffer[..read_bytes]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    return self.disconnect_with_error(format!(
                        "Session transport TCP read failed: {error}"
                    ));
                }
            }
        }

        let mut complete_lines = Vec::new();
        while let Some(newline_index) = self.inbound_buffer.iter().position(|byte| *byte == b'\n') {
            let mut raw_line: Vec<u8> = self.inbound_buffer.drain(..=newline_index).collect();
            if raw_line.last() == Some(&b'\n') {
                raw_line.pop();
            }
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
            if raw_line.is_empty() {
                continue;
            }
            let line = String::from_utf8(raw_line).map_err(|error| {
                format!("Session transport TCP received a non-UTF-8 line: {error}")
            })?;
            complete_lines.push(line);
        }
        if !complete_lines.is_empty() {
            transport.push_inbound_protocol_lines(complete_lines);
        }
        if closed_by_server && !self.inbound_buffer.is_empty() {
            self.inbound_buffer.clear();
            return Err(
                "Session transport TCP connection closed with an incomplete inbound line."
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl GuiSessionTransportDriver for GuiTcpSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        self.queue_outbound_lines(transport);
        self.flush_outbound_lines()?;
        self.drain_inbound_lines(transport)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuiPlayerLaunchRuntimeState {
    None,
    TestPlayer,
    ExplicitMpvIpc {
        ipc_path: String,
        ui_settings: Box<LegacySyncplayUiSettings>,
    },
    ManagedMpv(Box<ManagedMpvLaunchConfig>),
    UnsupportedConfiguredPlayer {
        player_path: String,
    },
}

impl GuiPlayerLaunchRuntimeState {
    fn default_unavailability_reason(&self) -> Option<String> {
        match self {
            Self::UnsupportedConfiguredPlayer { player_path } => Some(format!(
                "GUI-owned player launch currently supports mpv only; saved player path '{player_path}' was not started."
            )),
            Self::None | Self::TestPlayer | Self::ExplicitMpvIpc { .. } | Self::ManagedMpv(_) => {
                None
            }
        }
    }

    fn can_apply_mpv_ui_settings_in_place(&self, next: &Self) -> bool {
        match (self, next) {
            (
                Self::ExplicitMpvIpc {
                    ipc_path: current_path,
                    ..
                },
                Self::ExplicitMpvIpc {
                    ipc_path: next_path,
                    ..
                },
            ) => current_path == next_path,
            (Self::ManagedMpv(current), Self::ManagedMpv(next)) => {
                current.matches_process_target(next)
            }
            _ => false,
        }
    }

    fn mpv_ui_settings(&self) -> Option<&LegacySyncplayUiSettings> {
        match self {
            Self::ExplicitMpvIpc { ui_settings, .. } => Some(ui_settings),
            Self::ManagedMpv(config) => Some(&config.ui_settings),
            Self::None | Self::TestPlayer | Self::UnsupportedConfiguredPlayer { .. } => None,
        }
    }
}

struct GuiPersistedConfigRuntimeOwner {
    config_path: Option<PathBuf>,
    session: Option<Box<dyn GuiSessionRuntimeAdapter>>,
    session_projects_to_shell: bool,
    session_transport: Option<GuiQueuedSessionTransportHandle>,
    session_transport_driver: Option<Box<dyn GuiSessionTransportDriver>>,
    session_default_room: Option<String>,
    pending_room_change_request: Option<GuiPendingRoomChangeRequest>,
    startup_saved_connect_attempted: bool,
    player: Option<GuiOwnedPlayer>,
    player_launch_state: GuiPlayerLaunchRuntimeState,
    managed_mpv_process: Option<ManagedMpvProcessGuard>,
    player_unavailability_reason: Option<String>,
    player_local_file: Option<LocalFileUpdate>,
    player_position_seconds: Option<f64>,
    player_paused: Option<bool>,
    user_offset_seconds: f64,
}

impl GuiPersistedConfigRuntimeOwner {
    fn with_config_path(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path,
            session: None,
            session_projects_to_shell: false,
            session_transport: None,
            session_transport_driver: None,
            session_default_room: None,
            pending_room_change_request: None,
            startup_saved_connect_attempted: false,
            player: None,
            player_launch_state: GuiPlayerLaunchRuntimeState::None,
            managed_mpv_process: None,
            player_unavailability_reason: None,
            player_local_file: None,
            player_position_seconds: None,
            player_paused: None,
            user_offset_seconds: 0.0,
        }
    }

    fn with_config_path_and_startup_player(config_path: Option<PathBuf>) -> Self {
        Self::with_config_path_and_startup_player_lookup(config_path, &env_trimmed)
    }

    fn with_config_path_and_startup_player_lookup<F>(
        config_path: Option<PathBuf>,
        lookup: &F,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut owner = Self::with_config_path(config_path);
        let startup_settings = owner.load_startup_player_settings_from_config_path();
        owner.sync_player_from_lookup_and_settings(lookup, startup_settings.as_ref(), false);
        owner
    }

    fn load_startup_player_settings_from_config_path(&self) -> Option<StoredClientSettingsMvp> {
        self.config_path.as_ref().and_then(|path| {
            load_syncplay_ini_stored_client_settings_mvp_from_path(path)
                .ok()
                .flatten()
        })
    }

    fn clear_player_runtime_cache(&mut self) {
        self.player_local_file = None;
        self.player_position_seconds = None;
        self.player_paused = None;
    }

    fn detach_player(&mut self) {
        self.player = None;
        self.managed_mpv_process = None;
        self.clear_player_runtime_cache();
    }

    fn attach_player_from_launch_state(&mut self, launch_state: GuiPlayerLaunchRuntimeState) {
        self.detach_player();
        self.player_launch_state = launch_state.clone();
        self.player_unavailability_reason = launch_state.default_unavailability_reason();
        match launch_state {
            GuiPlayerLaunchRuntimeState::None => {}
            GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {}
            GuiPlayerLaunchRuntimeState::TestPlayer => {
                self.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
                self.player_unavailability_reason = None;
            }
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings,
            } => match MpvAdapter::with_json_ipc(&ipc_path) {
                Ok(mut adapter) => match apply_legacy_syncplay_ui_settings_to_mpv_adapter(
                    &mut adapter,
                    &ui_settings,
                ) {
                    Ok(()) => {
                        self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
                        self.player_unavailability_reason = None;
                    }
                    Err(error) => {
                        self.player_unavailability_reason = Some(format!(
                            "mpv JSON IPC attach succeeded at '{ipc_path}', but legacy GUI settings could not be applied: {error}"
                        ));
                    }
                },
                Err(error) => {
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC attach failed at '{ipc_path}': {error}"
                    ));
                }
            },
            GuiPlayerLaunchRuntimeState::ManagedMpv(config) => {
                match mpv_launch::spawn_managed_mpv_and_attach(&config) {
                    Ok((mut adapter, guard)) => {
                        match apply_legacy_syncplay_ui_settings_to_mpv_adapter(
                            &mut adapter,
                            &config.ui_settings,
                        ) {
                            Ok(()) => {
                                self.managed_mpv_process = Some(guard);
                                self.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
                                self.player_unavailability_reason = None;
                            }
                            Err(error) => {
                                self.player_unavailability_reason = Some(format!(
                                    "GUI-owned mpv started, but legacy GUI settings could not be applied: {error}"
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        self.player_unavailability_reason = Some(format!(
                            "GUI-owned mpv launch failed from saved player path '{}': {error}",
                            config.requested_player_path
                        ));
                    }
                }
            }
        }
    }

    fn configured_player_launch_state_from_lookup_and_settings<F>(
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
    ) -> Result<GuiPlayerLaunchRuntimeState, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        match env_flag_enabled_lookup(lookup, "SYNCPLAY_GUI_ENABLE_TEST_PLAYER") {
            Ok(true) => {
                return Ok(GuiPlayerLaunchRuntimeState::TestPlayer);
            }
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "SYNCPLAY_GUI_ENABLE_TEST_PLAYER could not be parsed: {error}"
                ));
            }
        }

        if let Some(ipc_path) = explicit_mpv_ipc_path_from_lookup(lookup) {
            let ui_settings =
                mpv_launch::legacy_syncplay_ui_settings_from_stored_settings(settings);
            return Ok(GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
                ipc_path,
                ui_settings: Box::new(ui_settings),
            });
        }

        Ok(
            match managed_mpv_settings_decision_from_settings(settings) {
                ManagedMpvSettingsDecision::NotConfigured => GuiPlayerLaunchRuntimeState::None,
                ManagedMpvSettingsDecision::UnsupportedConfiguredPlayer { player_path } => {
                    GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { player_path }
                }
                ManagedMpvSettingsDecision::Launch(config) => {
                    GuiPlayerLaunchRuntimeState::ManagedMpv(config)
                }
            },
        )
    }

    fn try_apply_mpv_ui_settings_in_place(
        &mut self,
        next_launch_state: &GuiPlayerLaunchRuntimeState,
    ) -> bool {
        if !self
            .player_launch_state
            .can_apply_mpv_ui_settings_in_place(next_launch_state)
        {
            return false;
        }
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return false;
        };
        let Some(ui_settings) = next_launch_state.mpv_ui_settings() else {
            return false;
        };
        if let Err(error) = apply_legacy_syncplay_ui_settings_to_mpv_adapter(player, ui_settings) {
            self.player_unavailability_reason =
                Some(format!("mpv legacy GUI settings reapply failed: {error}"));
            return false;
        }
        self.player_launch_state = next_launch_state.clone();
        self.player_unavailability_reason = None;
        true
    }

    fn sync_player_from_lookup_and_settings<F>(
        &mut self,
        lookup: &F,
        settings: Option<&StoredClientSettingsMvp>,
        force_relaunch: bool,
    ) where
        F: Fn(&str) -> Option<String>,
    {
        let next_launch_state =
            match Self::configured_player_launch_state_from_lookup_and_settings(lookup, settings) {
                Ok(state) => state,
                Err(error) => {
                    self.detach_player();
                    self.player_launch_state = GuiPlayerLaunchRuntimeState::None;
                    self.player_unavailability_reason = Some(error);
                    return;
                }
            };

        if !force_relaunch
            && self.player_launch_state == next_launch_state
            && (self.player.is_some() || self.player_unavailability_reason.is_some())
        {
            return;
        }
        if !force_relaunch && self.try_apply_mpv_ui_settings_in_place(&next_launch_state) {
            return;
        }
        self.attach_player_from_launch_state(next_launch_state);
    }

    fn ensure_configured_player_attached(&mut self) {
        if self.player.is_some() {
            return;
        }
        match self.player_launch_state.clone() {
            GuiPlayerLaunchRuntimeState::TestPlayer
            | GuiPlayerLaunchRuntimeState::ExplicitMpvIpc { .. }
            | GuiPlayerLaunchRuntimeState::ManagedMpv(_) => {
                self.attach_player_from_launch_state(self.player_launch_state.clone());
            }
            GuiPlayerLaunchRuntimeState::None
            | GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {}
        }
    }

    fn poll_managed_mpv_process(&mut self) {
        let exit_status = match self.managed_mpv_process.as_mut() {
            Some(guard) => match guard.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    self.detach_player();
                    self.player_unavailability_reason = Some(error);
                    return;
                }
            },
            None => return,
        };
        let Some(exit_status) = exit_status else {
            return;
        };

        let status_text = exit_status
            .code()
            .map(|code| format!("with exit code {code}"))
            .unwrap_or_else(|| "after an abnormal termination".to_owned());
        self.detach_player();
        self.player_unavailability_reason = Some(format!(
            "GUI-owned mpv exited {status_text}. Open media or save/reload configuration to relaunch it."
        ));
    }

    fn legacy_gui_qsettings_root(&self) -> Option<PathBuf> {
        self.config_path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    fn clear_gui_data(&mut self) -> Result<(), String> {
        if let Some(path) = self.config_path.as_ref() {
            clear_syncplay_ini_stored_client_settings_mvp_at_path(path).map_err(|error| {
                format!(
                    "failed clearing stored settings {}: {error}",
                    path.display()
                )
            })?;
        }
        if let Some(root) = self.legacy_gui_qsettings_root() {
            clear_legacy_gui_qsettings_files_at_root(&root)?;
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        Ok(())
    }

    #[allow(dead_code)]
    fn with_session_runtime(mut self, session: Box<dyn GuiSessionRuntimeAdapter>) -> Self {
        self.session = Some(session);
        self.session_projects_to_shell = true;
        self
    }

    fn with_session_default_room(mut self, room: impl Into<String>) -> Self {
        self.session_default_room = Some(room.into());
        self
    }

    #[allow(dead_code)]
    fn with_session_transport(
        mut self,
        session_transport: GuiQueuedSessionTransportHandle,
    ) -> Self {
        self.session_transport = Some(session_transport);
        self
    }

    #[allow(dead_code)]
    fn with_session_transport_driver(
        mut self,
        session_transport_driver: Box<dyn GuiSessionTransportDriver>,
    ) -> Self {
        self.session_transport_driver = Some(session_transport_driver);
        self
    }

    #[allow(dead_code)]
    fn with_client_core_chat_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        let room = room.into();
        let runtime_settings =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                username: Some(username.into()),
                room: Some(room.clone()),
                ..StoredClientSettingsMvp::default()
            });
        let session = Box::new(
            GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                runtime_settings.settings.username.unwrap_or_default(),
                runtime_settings.settings.room.unwrap_or_default(),
                runtime_settings.controlled_room_password_override,
            )?,
        );
        let session_transport = GuiQueuedSessionTransportHandle::default();
        Ok((
            self.with_session_runtime(session)
                .with_session_default_room(room)
                .with_session_transport(session_transport.clone()),
            session_transport,
        ))
    }

    #[allow(dead_code)]
    fn with_client_core_chat_loopback_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let (owner, _session_transport) =
            self.with_client_core_chat_session_runtime(username.clone(), room)?;
        Ok(
            owner.with_session_transport_driver(Box::new(GuiLoopbackSessionTransportDriver::new(
                username,
            ))),
        )
    }

    #[allow(dead_code)]
    fn with_client_core_chat_tcp_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
    ) -> Result<Self, String> {
        let (owner, _session_transport) =
            self.with_client_core_chat_session_runtime(username, room)?;
        Ok(owner.with_session_transport_driver(Box::new(
            GuiTcpSessionTransportDriver::connect_from_host_arg(host_arg.as_ref())?,
        )))
    }

    fn pump_session_transport_driver(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let Some(session_transport_driver) = self.session_transport_driver.as_mut() else {
            return;
        };
        if let Err(error) = session_transport_driver.pump(session_transport) {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: format!("Session transport driver pump failed: {error}"),
                }],
            );
        }
    }

    fn drain_session_transport_inbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let inbound_protocol_lines = session_transport.drain_inbound_protocol_lines();
        if inbound_protocol_lines.is_empty() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        for inbound_protocol_line in inbound_protocol_lines {
            if let Err(error) = session.apply_message_json(&inbound_protocol_line) {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Inbound session transport message apply failed: {error}"),
                    }],
                );
            }
        }
    }

    fn drain_session_runtime_actions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if !self.session_projects_to_shell {
            if let Some(session) = self.session.as_mut() {
                let _ = session.drain_gui_actions(projected_state);
            }
            return;
        }
        let actions = {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            session.drain_gui_actions(projected_state)
        };
        let actions = self.augment_runtime_actions_for_room_transitions(projected_state, actions);
        self.emit_gui_actions_to_attached_player(&actions);
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn flush_session_transport_outbound(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(session_transport) = self.session_transport.as_ref() else {
            return;
        };
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match session.flush_outbound_protocol_lines() {
            Ok(outbound_protocol_lines) => {
                if !outbound_protocol_lines.is_empty() {
                    session_transport.push_outbound_protocol_lines(outbound_protocol_lines);
                }
            }
            Err(error) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: format!("Outbound session transport flush failed: {error}"),
                    }],
                );
            }
        }
    }

    fn push_runtime_unavailable(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    fn default_room_for_legacy_fallback(
        &self,
        projected_state: &SyncplayGuiShellAppState,
    ) -> String {
        self.session_default_room
            .clone()
            .or_else(|| {
                projected_state
                    .saved_session_connect_target()
                    .map(|target| target.room)
            })
            .unwrap_or_else(|| {
                Self::detached_runtime_settings_for_state(projected_state)
                    .settings
                    .room
                    .unwrap_or_default()
            })
    }

    fn augment_runtime_actions_for_room_transitions(
        &mut self,
        projected_state: &SyncplayGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) -> Vec<GuiShellAction> {
        let mut current_room = projected_state.main_window.room_name.clone();
        let mut augmented_actions = Vec::with_capacity(actions.len());
        for action in actions {
            match action {
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                    let next_room = snapshot.room_name.clone();
                    let room_transition_actions =
                        self.room_transition_confirmation_actions(&current_room, &next_room);
                    current_room = next_room;
                    augmented_actions
                        .push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
                    augmented_actions.extend(room_transition_actions);
                }
                other => augmented_actions.push(other),
            }
        }
        augmented_actions
    }

    fn room_transition_confirmation_actions(
        &mut self,
        previous_room: &str,
        next_room: &str,
    ) -> Vec<GuiShellAction> {
        if previous_room == next_room {
            return Vec::new();
        }

        let Some(request) = self.pending_room_change_request.take() else {
            return Vec::new();
        };

        let (level, message) = match request {
            GuiPendingRoomChangeRequest::Join { .. } => (
                GuiTransientNotificationLevel::Success,
                format!("Room joined: {next_room}."),
            ),
            GuiPendingRoomChangeRequest::ReturnToDefault { .. } => (
                GuiTransientNotificationLevel::Info,
                format!("Returned to default room: {next_room}."),
            ),
        };

        vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: next_room.to_owned(),
            },
            GuiShellAction::PushTransientNotification {
                level,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn request_room_join_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        room: String,
    ) {
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Changing rooms requires an active session runtime.".to_owned(),
            );
            return;
        };

        match session.set_room(room.clone()) {
            Ok(()) => {
                self.pending_room_change_request = Some(GuiPendingRoomChangeRequest::Join {
                    requested_room: room,
                });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }

    fn request_room_leave_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let previous_room = projected_state.main_window.room_name.clone();
        let default_room = self.default_room_for_legacy_fallback(projected_state);
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Returning to the default room requires an active session runtime.".to_owned(),
            );
            return;
        };

        match session.set_room_with_legacy_fallback(default_room) {
            Ok(()) => {
                self.pending_room_change_request =
                    Some(GuiPendingRoomChangeRequest::ReturnToDefault { previous_room });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }

    fn open_media_unavailable_message(&self, selected_paths: &[String]) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media requires a playback runtime connection; the selected file was not opened."
                .to_owned()
        } else {
            format!(
                "Opening media requires a playback runtime connection; {} selected files were not opened.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    fn shared_playlist_open_unavailable_message(&self, selected_paths: &[String]) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                .to_owned()
        } else {
            format!(
                "Opening media into the shared playlist requires a session or playback runtime connection; {} selected files were not opened or queued.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    fn shared_playlist_session_unavailable_message(&self) -> String {
        "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
            .to_owned()
    }

    fn shared_playlist_entry_for_media_path(path: &str) -> Option<String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains("://") {
            return Some(trimmed.to_owned());
        }
        Some(
            Path::new(trimmed)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(trimmed)
                .to_owned(),
        )
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        let lower_path = path.to_ascii_lowercase();
        if !(lower_path.ends_with(".txt")
            || lower_path.ends_with(".m3u")
            || lower_path.ends_with(".m3u8"))
        {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = contents
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    fn shared_playlist_open_dispatch_for_paths(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| Self::shared_playlist_entry_for_media_path(path))
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
            imported_from_file: false,
        })
    }

    fn shared_playlist_open_success_message(dispatch: &GuiSharedPlaylistOpenDispatch) -> String {
        let entry_count = dispatch.playlist_entries.len();
        if dispatch.imported_from_file {
            if entry_count == 1 {
                "Imported 1 entry into the shared playlist.".to_owned()
            } else {
                format!("Imported {entry_count} entries into the shared playlist.")
            }
        } else if entry_count == 1 {
            "Loaded 1 selected media entry into the shared playlist.".to_owned()
        } else {
            format!("Loaded {entry_count} selected media entries into the shared playlist.")
        }
    }

    fn seek_unavailable_message(&self, offset_seconds: f64) -> String {
        let base = format!(
            "Playback seek requires a playback runtime connection; the {offset_seconds} second request was not applied."
        );
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    fn toggle_pause_unavailable_message(&self) -> String {
        let base =
            "Playback toggle requires a playback runtime connection; the pause request was not applied."
                .to_owned();
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    fn send_chat_unavailable_message(&self) -> String {
        "Chat sending requires a session runtime connection; the message was not sent.".to_owned()
    }

    fn detached_runtime_settings_for_state(
        state: &SyncplayGuiShellAppState,
    ) -> StoredClientSettingsRuntimeSnapshot {
        stored_client_settings_runtime_snapshot_legacy_compatible(
            &state.configuration.to_stored_settings(),
        )
    }

    fn ensure_detached_client_core_chat_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<(), String> {
        if self.session.is_none() {
            let runtime_settings = Self::detached_runtime_settings_for_state(state);
            self.session_default_room = runtime_settings.settings.room.clone();
            self.session = Some(Box::new(
                GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
                    runtime_settings.settings.username.unwrap_or_default(),
                    runtime_settings.settings.room.unwrap_or_default(),
                    runtime_settings.controlled_room_password_override,
                )?,
            ));
            self.session_projects_to_shell = false;
        }
        if self.session_transport.is_none() {
            self.session_transport = Some(GuiQueuedSessionTransportHandle::default());
        }
        self.sync_detached_session_preferences_and_player_state(state)?;
        Ok(())
    }

    fn sync_detached_session_preferences_and_player_state(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<(), String> {
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        session.set_autoplay_enabled(state.main_window.autoplay_active)?;
        session.set_autoplay_threshold(state.main_window.autoplay_threshold)?;
        Ok(())
    }

    fn refresh_public_servers_without_session(
        _current_servers: Vec<(String, String)>,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(refreshed_servers) =
            GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_env()?
        {
            return Ok(refreshed_servers);
        }
        #[cfg(test)]
        {
            Ok(
                GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                    _current_servers,
                ),
            )
        }
        #[cfg(not(test))]
        {
            let refreshed_servers = remote_services::fetch_public_servers(Some("en"))?;
            Ok(
                GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                    refreshed_servers,
                ),
            )
        }
    }

    fn detached_missing_media_target(&self, state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(local_file) = self.player_local_file.as_ref() {
            if let Some(path) = local_file
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                return Some(path.to_owned());
            }
            let name = local_file.name.trim();
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }

        if let Some(index) = state.selection.selected_main_window_playlist
            && let Some(row) = state.main_window.playlist.get(index)
        {
            let label = row.label.trim();
            if !label.is_empty() {
                return Some(label.to_owned());
            }
        }

        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.trim())
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
    }

    fn detached_missing_media_target_file_name(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<String, String> {
        let Some(target) = self.detached_missing_media_target(state) else {
            return Err(
                "Detached GUI missing-media search could not determine a target file from the current player or playlist state."
                    .to_owned(),
            );
        };
        if target.contains("://") {
            return Err(
                "Detached GUI missing-media search does not support URL-based media targets."
                    .to_owned(),
            );
        }
        let Some(file_name) = Path::new(&target)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return Err(
                "Detached GUI missing-media search could not derive a file name from the current player or playlist state."
                    .to_owned(),
            );
        };
        let file_name = file_name.trim();
        if file_name.is_empty() {
            return Err(
                "Detached GUI missing-media search could not derive a non-empty file name from the current player or playlist state."
                    .to_owned(),
            );
        }
        Ok(file_name.to_owned())
    }

    fn search_missing_media_without_session(
        &self,
        state: &SyncplayGuiShellAppState,
        directories: Vec<String>,
    ) -> Result<Option<String>, String> {
        let target_file_name = self.detached_missing_media_target_file_name(state)?;
        for directory in directories {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                    &target_file_name,
                    Path::new(trimmed),
                )?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    fn session_active(&self) -> bool {
        self.session_projects_to_shell
    }

    fn sessionless_main_window_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> MainWindowRuntimeSnapshot {
        let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        let player_attached = self.player.is_some();
        snapshot.can_toggle_pause = player_attached;
        snapshot.can_seek = player_attached;
        snapshot.can_undo_seek = false;
        snapshot.can_set_offset = player_attached;
        snapshot.can_toggle_autoplay = true;
        snapshot.can_adjust_autoplay_threshold = true;
        snapshot.can_manage_playlist = player_attached && snapshot.shared_playlist_enabled;
        if !snapshot.shared_playlist_enabled {
            snapshot.playlist = self.player_local_file_playlist_entries();
        }
        if player_attached && let Some(paused) = self.player_paused {
            snapshot.playback_paused = paused;
        }
        snapshot.autoplay_countdown_seconds = None;
        snapshot.user_offset_seconds = self.user_offset_seconds;
        snapshot
    }

    fn sessionless_menu_dialog_runtime_snapshot(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Option<MenuDialogRuntimeSnapshot> {
        let settings = state.configuration.to_stored_settings();
        let desired_show_chat_enabled = settings.chat_input_enabled.unwrap_or(false)
            || settings.chat_output_enabled.unwrap_or(false);
        let desired_show_playlist_enabled = settings.shared_playlist_enabled.unwrap_or(false);
        let mut action_overrides = Vec::new();
        for (section_title, action_label, enabled) in [
            ("Window", "Show Chat", desired_show_chat_enabled),
            ("Window", "Show Playlist", desired_show_playlist_enabled),
            ("Advanced", "Create Controlled Room", false),
            ("Advanced", "Identify As Controller", false),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == section_title)
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title,
                    action_label,
                    enabled,
                });
            }
        }
        if action_overrides.is_empty() {
            return None;
        }
        Some(MenuDialogRuntimeSnapshot {
            action_overrides,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        })
    }

    fn sessionless_projection_actions(
        &self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        let main_window_snapshot = self.sessionless_main_window_snapshot(state);
        if main_window_snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window) {
            actions.push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                main_window_snapshot,
            ));
        }
        if let Some(menu_snapshot) = self.sessionless_menu_dialog_runtime_snapshot(state) {
            actions.push(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
                menu_snapshot,
            ));
        }
        actions
    }

    fn push_runtime_error_notification(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        message: String,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message,
            }],
        );
    }

    fn complete_saved_server_connect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        clear_pending: bool,
    ) {
        let Some(target) = projected_state.saved_session_connect_target() else {
            let message =
                "Configured server connect requires a saved host and a valid port.".to_owned();
            if clear_pending {
                self.clear_pending_operation_with_runtime_error(handle, projected_state, message);
            } else {
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return;
        };
        let transport_driver = match GuiTcpSessionTransportDriver::connect_from_host_arg(
            &target.address,
        ) {
            Ok(driver) => driver,
            Err(error) => {
                let message = format!(
                    "Configured server connect through the detached session runtime failed: {error}"
                );
                if clear_pending {
                    self.clear_pending_operation_with_runtime_error(
                        handle,
                        projected_state,
                        message,
                    );
                } else {
                    Self::push_runtime_error_notification(handle, projected_state, message);
                }
                return;
            }
        };
        let default_room = target.room.clone();
        let session = match GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
            target.username,
            target.room,
            target.controlled_room_password_override,
        ) {
            Ok(session) => session,
            Err(error) => {
                let message = format!(
                    "Configured server connect through the detached session runtime failed: {error}"
                );
                if clear_pending {
                    self.clear_pending_operation_with_runtime_error(
                        handle,
                        projected_state,
                        message,
                    );
                } else {
                    Self::push_runtime_error_notification(handle, projected_state, message);
                }
                return;
            }
        };

        self.session = Some(Box::new(session));
        self.session_projects_to_shell = true;
        self.session_default_room = Some(default_room);
        self.pending_room_change_request = None;
        if self.session_transport.is_none() {
            self.session_transport = Some(GuiQueuedSessionTransportHandle::default());
        }
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session_transport_driver = Some(Box::new(transport_driver));

        let mut actions = self.sessionless_projection_actions(projected_state);
        if clear_pending {
            actions.push(GuiShellAction::CompleteSavedServerConnect);
        }
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn complete_session_disconnect_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if let Some(session_transport) = self.session_transport.as_ref() {
            session_transport.clear_protocol_lines();
        }
        self.session = None;
        self.session_projects_to_shell = false;
        self.session_transport = None;
        self.session_transport_driver = None;
        self.session_default_room = None;
        self.pending_room_change_request = None;

        let mut actions = self.sessionless_projection_actions(projected_state);
        actions.push(GuiShellAction::CompleteSessionDisconnect);
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn push_actions_and_project(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) {
        if actions.is_empty() {
            return;
        }
        handle.push_actions(actions.clone());
        for action in actions {
            let _ = projected_state.apply(action);
        }
    }

    fn clear_pending_operation_with_runtime_error(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        message: String,
    ) {
        let mut cleared_state = projected_state.clone();
        cleared_state.pending_operation = None;
        let actions = vec![
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: self
                    .command_availability_for_runtime_state(&cleared_state, self.player.is_some()),
                pending_operation: None,
            }),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message,
            },
        ];
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    fn clear_pending_operation_runtime_state(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let mut cleared_state = projected_state.clone();
        cleared_state.pending_operation = None;
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
                GuiCommandRuntimeSnapshot {
                    command_availability: self.command_availability_for_runtime_state(
                        &cleared_state,
                        self.player.is_some(),
                    ),
                    pending_operation: None,
                },
            )],
        );
    }

    fn push_player_success(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    fn push_player_error(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    fn open_media_files_through_attached_player_result(
        &mut self,
        paths: &[String],
    ) -> Option<Result<String, String>> {
        if paths.is_empty() || self.player.is_none() {
            return None;
        }

        let selected_path = paths[0].clone();
        let (player_name, open_result) = {
            let player = self.player.as_mut().expect("player should exist");
            (player.name(), player.open_file(&selected_path))
        };
        Some(match open_result {
            Ok(()) => {
                self.player_local_file =
                    Some(Self::placeholder_local_file_for_path(&selected_path));
                self.player_position_seconds = Some(0.0);
                self.refresh_player_state();
                if paths.len() == 1 {
                    Ok(format!(
                        "Opened media file through the attached {player_name} player: {selected_path}."
                    ))
                } else {
                    Ok(format!(
                        "Opened the first selected media file through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                        paths.len() - 1
                    ))
                }
            }
            Err(error) => Err(format!(
                "Opening media through the attached {player_name} player failed: {error}"
            )),
        })
    }

    fn open_media_files_through_attached_player(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        match self.open_media_files_through_attached_player_result(&paths) {
            Some(Ok(message)) => Self::push_player_success(handle, message),
            Some(Err(message)) => Self::push_player_error(handle, message),
            None => {}
        }
    }

    fn resolve_main_window_user_media_target(
        &self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<Option<String>, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(None);
        };
        if browser_is_url(&target) {
            return Ok(Some(target.to_owned()));
        }

        let target_path = Path::new(&target);
        if target_path.is_file() {
            return Ok(Some(target.to_owned()));
        }

        if let Some(local_path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
        {
            let local_path = Path::new(local_path);
            let matches_local_file = local_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(&target));
            if matches_local_file && local_path.is_file() {
                return Ok(Some(local_path.to_string_lossy().into_owned()));
            }
            if let Some(parent) = local_path.parent()
                && let Some(found_path) =
                    GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                        &target, parent,
                    )?
            {
                return Ok(Some(found_path));
            }
        }

        let settings = state.configuration.to_stored_settings();
        for directory in settings.media_search_directories.unwrap_or_default() {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                    &target,
                    Path::new(trimmed),
                )?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    fn open_main_window_user_media_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!("Could not find a local path for user media: {target}."),
                    );
                    return;
                }
                Err(error) => {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!("Resolving user media '{target}' failed: {error}"),
                    );
                    return;
                }
            };

        self.ensure_configured_player_attached();
        if self.player.is_some() {
            self.open_media_files_through_attached_player(handle, vec![resolved_target]);
        } else {
            Self::push_runtime_unavailable(
                handle,
                self.open_media_unavailable_message(&[resolved_target]),
            );
        }
    }

    fn open_system_file_browser_for_path(path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err(format!(
                "Could not open a containing folder for '{}': no parent directory exists.",
                path.display()
            ));
        };

        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg(parent);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(parent);
            command
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(parent);
            command
        };

        command.spawn().map_err(|error| {
            format!(
                "Opening the containing folder for '{}' failed: {error}",
                path.display()
            )
        })?;
        Ok(())
    }

    fn open_main_window_user_containing_folder_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!("Could not find a local path for user media: {target}."),
                    );
                    return;
                }
                Err(error) => {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!("Resolving user media '{target}' failed: {error}"),
                    );
                    return;
                }
            };

        if browser_is_url(&resolved_target) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Cannot open a containing folder for the stream URL: {resolved_target}."),
            );
            return;
        }

        if let Err(error) = Self::open_system_file_browser_for_path(Path::new(&resolved_target)) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }

    fn open_media_files_through_shared_playlist_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        self.ensure_configured_player_attached();
        let selected_paths = paths
            .into_iter()
            .filter_map(|path| normalized_editable_text(&path))
            .collect::<Vec<_>>();
        if selected_paths.is_empty() {
            return;
        }

        let dispatch = match Self::shared_playlist_open_dispatch_for_paths(selected_paths.clone()) {
            Ok(dispatch) => dispatch,
            Err(error) => {
                Self::push_runtime_unavailable(handle, error);
                return;
            }
        };

        let player_result = dispatch.player_paths.as_ref().and_then(|player_paths| {
            self.open_media_files_through_attached_player_result(player_paths)
        });

        if self.session.is_none() {
            match player_result {
                Some(Ok(message)) => {
                    let warning = self.shared_playlist_session_unavailable_message();
                    handle.push_actions([
                        GuiShellAction::SwitchView(GuiShellView::MainWindow),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Success,
                            message: message.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(message),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Warning,
                            message: warning.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(warning),
                    ]);
                }
                Some(Err(message)) => {
                    let warning = self.shared_playlist_session_unavailable_message();
                    handle.push_actions([
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Warning,
                            message: warning.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(warning),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: message.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(message),
                    ]);
                }
                None => Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message(&selected_paths),
                ),
            }
            return;
        }

        let session_result = self
            .session
            .as_mut()
            .expect("session should exist")
            .replace_playlist(
                dispatch.playlist_entries.clone(),
                (!dispatch.playlist_entries.is_empty()).then_some(0),
            );
        let session_success = session_result.is_ok();
        let player_success = player_result.as_ref().is_some_and(Result::is_ok);

        let mut actions = Vec::new();
        if session_success || player_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::MainWindow));
        }
        if session_success && !player_success {
            let message = Self::shared_playlist_open_success_message(&dispatch);
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        match player_result {
            Some(Ok(message)) => {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: message.clone(),
                });
                actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
            }
            Some(Err(message)) => {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: message.clone(),
                });
                actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
            }
            None => {}
        }
        if let Err(error) = session_result {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
        }
        handle.push_actions(actions);
    }

    fn emit_gui_actions_to_attached_player(&mut self, actions: &[GuiShellAction]) {
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return;
        };
        let mut already_emitted_osd_messages = BTreeSet::new();
        for action in actions {
            match action {
                GuiShellAction::PushChatMessage { sender, message } => {
                    if let Err(error) =
                        player.show_syncplay_legacy_chat_message(&format!("<{sender}> {message}"))
                    {
                        eprintln!(
                            "warning: failed to display GUI chat notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::PushTransientNotification { level, message } => {
                    already_emitted_osd_messages.insert(message.clone());
                    let kind = match level {
                        GuiTransientNotificationLevel::Info
                        | GuiTransientNotificationLevel::Success => {
                            LegacySyncplayOsdKind::Notification
                        }
                        GuiTransientNotificationLevel::Warning
                        | GuiTransientNotificationLevel::Error => LegacySyncplayOsdKind::Alert,
                    };
                    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
                        eprintln!(
                            "warning: failed to display GUI notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if already_emitted_osd_messages.insert(message.clone()) =>
                {
                    if let Err(error) = player
                        .show_syncplay_legacy_message(message, LegacySyncplayOsdKind::Notification)
                    {
                        eprintln!(
                            "warning: failed to display GUI system-chat event via mpv OSD: {error}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn drain_player_chat_input(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if self.session.is_none() {
            return;
        }

        let mut errors = Vec::new();
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
            let send_result = self
                .session
                .as_mut()
                .expect("session should exist when draining player chat")
                .send_chat_message(message.clone());
            if let Err(error) = send_result {
                errors.push(format!(
                    "Chat input from the attached player could not be sent: {error}"
                ));
            }
        }

        if !errors.is_empty() {
            Self::push_actions_and_project(
                handle,
                projected_state,
                errors
                    .into_iter()
                    .flat_map(|message| {
                        [
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: message.clone(),
                            },
                            GuiShellAction::AnnounceSystemChatEvent(message),
                        ]
                    })
                    .collect(),
            );
        }
    }

    fn refresh_player_state(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        while let Some(update) = player.take_playback_telemetry_update() {
            if let Some(paused) = update.paused {
                self.player_paused = Some(paused);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds);
            }
        }
        while let Some(update) = player.take_local_file_update() {
            self.player_local_file = Some(update);
            if self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
    }

    fn sync_manual_seek_into_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))?;
        let _ = session.record_manual_seek_to_position(target_position_seconds)?;
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(())
    }

    fn sync_playback_pause_into_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let _ = session.set_playback_paused(target_paused)?;
        session.sync_local_playback_telemetry(Some(target_paused), self.player_position_seconds)?;
        Ok(())
    }

    fn undo_seek_target_position_from_detached_session(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        if !session.undo_seek()? {
            return Ok(None);
        }
        let target = session.local_position_seconds();
        session.sync_local_playback_telemetry(self.player_paused, target)?;
        Ok(target)
    }

    fn format_local_file_playlist_entry(local_file: &LocalFileUpdate) -> String {
        let mut details = Vec::new();
        if let Some(duration_seconds) = local_file.duration_seconds {
            details.push(format!("{duration_seconds:.3}s"));
        }
        if let Some(size_bytes) = local_file.size_bytes {
            details.push(format!("{size_bytes} bytes"));
        }
        if details.is_empty() {
            local_file.name.clone()
        } else {
            format!("{} [{}]", local_file.name, details.join(", "))
        }
    }

    fn player_local_file_playlist_entries(&self) -> Vec<String> {
        self.player_local_file
            .as_ref()
            .map(Self::format_local_file_playlist_entry)
            .into_iter()
            .collect()
    }

    fn command_availability_for_runtime_state(
        &self,
        state: &SyncplayGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        let settings = state.configuration.to_stored_settings();
        let busy = state.pending_operation.is_some();
        let command_availability = GuiCommandAvailabilityState {
            can_save_configuration: !busy && state.validation.issues.is_empty(),
            can_reset_configuration: !busy && state.has_unsaved_configuration_changes(),
            can_reload_configuration: !busy,
            can_connect_saved_server: !busy && state.saved_session_connect_target().is_some(),
            can_disconnect_session: !busy && self.session_active(),
            can_connect_public_server: !busy && state.public_servers.can_connect,
            can_refresh_public_servers: !busy && state.public_servers.can_refresh,
            can_search_missing_media: !busy && state.media_search.can_search_missing_media,
            can_toggle_pause: !busy && player_attached,
            can_send_chat_message: !busy && settings.chat_input_enabled.unwrap_or(false),
        };
        if let Some(session) = self.session.as_ref() {
            session.adjust_command_availability(state, command_availability)
        } else {
            command_availability
        }
    }

    fn placeholder_local_file_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        LocalFileUpdate::new(name).with_path(path.to_owned())
    }

    fn sync_player_runtime_state(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SyncplayGuiShellAppState,
    ) {
        let player_attached = self.player.is_some();

        let mut desired_main_window =
            MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
        let mut main_window_changed = false;

        if desired_main_window.can_toggle_pause != player_attached {
            desired_main_window.can_toggle_pause = player_attached;
            main_window_changed = true;
        }
        if desired_main_window.can_seek != player_attached {
            desired_main_window.can_seek = player_attached;
            main_window_changed = true;
        }
        if desired_main_window.can_set_offset != player_attached {
            desired_main_window.can_set_offset = player_attached;
            main_window_changed = true;
        }
        let can_manage_playlist = self
            .session
            .as_ref()
            .map(|session| {
                desired_main_window.shared_playlist_enabled && session.playlist_control_available()
            })
            .unwrap_or(player_attached && desired_main_window.shared_playlist_enabled);
        if desired_main_window.can_manage_playlist != can_manage_playlist {
            desired_main_window.can_manage_playlist = can_manage_playlist;
            main_window_changed = true;
        }
        if !desired_main_window.shared_playlist_enabled {
            let desired_playlist = self.player_local_file_playlist_entries();
            if desired_main_window.playlist != desired_playlist {
                desired_main_window.playlist = desired_playlist;
                main_window_changed = true;
            }
        }
        if player_attached
            && let Some(paused) = self.player_paused
            && desired_main_window.playback_paused != paused
        {
            desired_main_window.playback_paused = paused;
            main_window_changed = true;
        }
        if (desired_main_window.user_offset_seconds - self.user_offset_seconds).abs() > f64::EPSILON
        {
            desired_main_window.user_offset_seconds = self.user_offset_seconds;
            main_window_changed = true;
        }
        if main_window_changed {
            handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                desired_main_window,
            ));
        }

        let mut action_overrides = Vec::new();
        for (action_label, enabled) in [
            ("Play", player_attached),
            ("Pause", player_attached),
            ("Toggle Pause", player_attached),
            ("Seek", player_attached),
            (
                "Undo Seek",
                state.pending_operation.is_none() && state.main_window.playback.can_undo_seek,
            ),
            (
                "Playlist Actions",
                self.session
                    .as_ref()
                    .map(|session| {
                        state.main_window.shared_playlist_enabled
                            && session.playlist_control_available()
                    })
                    .unwrap_or(player_attached && state.main_window.shared_playlist_enabled),
            ),
        ] {
            let current_enabled = state
                .menus
                .sections
                .iter()
                .find(|section| section.title == "Playback")
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .find(|action| action.label == action_label)
                })
                .map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label,
                    enabled,
                });
            }
        }
        let current_offset_enabled = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Advanced")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Set Offset")
            })
            .map(|action| action.enabled);
        let desired_offset_enabled = state.pending_operation.is_none() && player_attached;
        if current_offset_enabled
            .is_some_and(|current_enabled| current_enabled != desired_offset_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Advanced",
                action_label: "Set Offset",
                enabled: desired_offset_enabled,
            });
        }
        if !action_overrides.is_empty() {
            handle.push_action(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
                MenuDialogRuntimeSnapshot {
                    action_overrides,
                    tls_prompt_expected: state.menus.tls_prompt_expected,
                    update_notice_expected: state.menus.update_notice_expected,
                    about_dialog_available: state.menus.about_dialog_available,
                },
            ));
        }

        let desired_command_availability =
            self.command_availability_for_runtime_state(state, player_attached);
        if state.commands != desired_command_availability {
            handle.push_action(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
                GuiCommandRuntimeSnapshot {
                    command_availability: desired_command_availability,
                    pending_operation: state.pending_operation.as_ref().map(|pending| pending.kind),
                },
            ));
        }
    }
}

impl Default for GuiPersistedConfigRuntimeOwner {
    fn default() -> Self {
        let mut owner = Self::with_config_path_and_startup_player(
            resolve_syncplay_gui_config_path_legacy_compatible(),
        );
        if owner.player.is_none() && owner.player_unavailability_reason.is_none() {
            owner.player_unavailability_reason = Some(
                "Set playerPath to mpv in GUI settings, or set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
                    .to_owned(),
            );
        }
        owner
    }
}

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SyncplayGuiShellAppState) {
        self.poll_managed_mpv_process();
        self.refresh_player_state();
        let mut projected_state = state.clone();
        if let Err(error) =
            self.sync_detached_session_preferences_and_player_state(&projected_state)
        {
            Self::push_runtime_unavailable(handle, error);
        }
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions(handle, &mut projected_state);
        self.drain_player_chat_input(handle, &mut projected_state);
        self.flush_session_transport_outbound(handle, &mut projected_state);
        self.pump_session_transport_driver(handle, &mut projected_state);
        self.drain_session_transport_inbound(handle, &mut projected_state);
        self.drain_session_runtime_actions(handle, &mut projected_state);
        self.drain_player_chat_input(handle, &mut projected_state);
        if !self.startup_saved_connect_attempted {
            self.startup_saved_connect_attempted = true;
            if projected_state.pending_operation.is_none()
                && !self.session_active()
                && projected_state.saved_session_connect_target().is_some()
            {
                self.complete_saved_server_connect_runtime(handle, &mut projected_state, false);
                self.flush_session_transport_outbound(handle, &mut projected_state);
                self.pump_session_transport_driver(handle, &mut projected_state);
                self.drain_session_transport_inbound(handle, &mut projected_state);
                self.drain_session_runtime_actions(handle, &mut projected_state);
                self.drain_player_chat_input(handle, &mut projected_state);
            }
        }
        for request in handle.drain_requests() {
            match request {
                GuiRuntimeRequest::OpenMediaFiles {
                    paths,
                    load_into_shared_playlist: true,
                } => {
                    self.open_media_files_through_shared_playlist_runtime(handle, paths);
                }
                GuiRuntimeRequest::OpenMediaFiles {
                    paths,
                    load_into_shared_playlist: false,
                } => {
                    if paths.is_empty() {
                        continue;
                    }
                    self.ensure_configured_player_attached();
                    if self.player.is_some() {
                        self.open_media_files_through_attached_player(handle, paths);
                    } else {
                        Self::push_runtime_unavailable(
                            handle,
                            self.open_media_unavailable_message(&paths),
                        );
                    }
                }
                GuiRuntimeRequest::OpenMainWindowUserMedia(target) => {
                    self.open_main_window_user_media_runtime(handle, &mut projected_state, target);
                }
                GuiRuntimeRequest::OpenMainWindowUserContainingFolder(target) => {
                    self.open_main_window_user_containing_folder_runtime(
                        handle,
                        &mut projected_state,
                        target,
                    );
                }
                GuiRuntimeRequest::UndoSeek => {
                    self.refresh_player_state();
                    self.ensure_configured_player_attached();
                    if self.player.is_none() {
                        Self::push_runtime_unavailable(
                            handle,
                            "Playback undo seek requires a playback runtime connection.".to_owned(),
                        );
                        continue;
                    }
                    match self.undo_seek_target_position_from_detached_session(&projected_state) {
                        Ok(Some(target_position_seconds)) => {
                            let (player_name, undo_result) = {
                                let player = self.player.as_mut().expect("player should exist");
                                (player.name(), player.set_position(target_position_seconds))
                            };
                            match undo_result {
                                Ok(()) => {
                                    self.player_position_seconds = Some(target_position_seconds);
                                    self.refresh_player_state();
                                    Self::push_player_success(
                                        handle,
                                        format!(
                                            "Undo seek applied via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                        ),
                                    );
                                }
                                Err(error) => Self::push_player_error(
                                    handle,
                                    format!(
                                        "Playback undo seek through the attached {player_name} player failed: {error}"
                                    ),
                                ),
                            }
                        }
                        Ok(None) => Self::push_player_error(
                            handle,
                            "Playback undo seek is unavailable because no earlier seek target is recorded."
                                .to_owned(),
                        ),
                        Err(error) => Self::push_player_error(handle, error),
                    }
                }
                GuiRuntimeRequest::SetOffset(command) => {
                    self.refresh_player_state();
                    self.ensure_configured_player_attached();
                    if self.player.is_none() {
                        Self::push_runtime_unavailable(
                            handle,
                            "Playback offset changes require a playback runtime connection."
                                .to_owned(),
                        );
                        continue;
                    }
                    let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
                    let dispatch = plan_local_offset_runtime_dispatch_legacy_compatible(
                        self.user_offset_seconds,
                        previous_position_seconds,
                        &command,
                        None,
                    );
                    let Some(PlannedLocalRuntimeAction::SeekToPosition(target_position_seconds)) =
                        dispatch.action
                    else {
                        continue;
                    };
                    let (player_name, offset_result) = {
                        let player = self.player.as_mut().expect("player should exist");
                        (player.name(), player.set_position(target_position_seconds))
                    };
                    match offset_result {
                        Ok(()) => {
                            self.player_position_seconds = Some(target_position_seconds);
                            self.user_offset_seconds = dispatch
                                .updated_user_offset_seconds
                                .unwrap_or(self.user_offset_seconds);
                            self.refresh_player_state();
                            if let Err(error) = self.sync_manual_seek_into_detached_session(
                                &projected_state,
                                previous_position_seconds,
                                target_position_seconds,
                            ) {
                                Self::push_player_error(handle, error);
                            }
                            let message = dispatch.line_to_emit.unwrap_or_else(|| {
                                localized_current_offset_message_legacy_compatible(
                                    self.user_offset_seconds,
                                    None,
                                )
                            });
                            Self::push_player_success(
                                handle,
                                format!("{message} Applied via the attached {player_name} player."),
                            );
                        }
                        Err(error) => Self::push_player_error(
                            handle,
                            format!(
                                "Playback offset change through the attached {player_name} player failed: {error}"
                            ),
                        ),
                    }
                }
                GuiRuntimeRequest::SetAutoplayEnabled(enabled) => {
                    if let Err(error) =
                        self.ensure_detached_client_core_chat_session(&projected_state)
                    {
                        Self::push_player_error(handle, error);
                        continue;
                    }
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.set_autoplay_enabled(enabled)
                    {
                        Self::push_player_error(handle, error);
                    }
                }
                GuiRuntimeRequest::SetAutoplayThreshold(threshold) => {
                    if let Err(error) =
                        self.ensure_detached_client_core_chat_session(&projected_state)
                    {
                        Self::push_player_error(handle, error);
                        continue;
                    }
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.set_autoplay_threshold(threshold)
                    {
                        Self::push_player_error(handle, error);
                    }
                }
                GuiRuntimeRequest::SeekOffset(offset_seconds) => {
                    self.refresh_player_state();
                    self.ensure_configured_player_attached();
                    if self.player.is_some() {
                        let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
                        let target_position_seconds =
                            (previous_position_seconds + offset_seconds).max(0.0);
                        let (player_name, seek_result) = {
                            let player = self.player.as_mut().expect("player should exist");
                            (player.name(), player.set_position(target_position_seconds))
                        };
                        match seek_result {
                            Ok(()) => {
                                self.player_position_seconds = Some(target_position_seconds);
                                self.refresh_player_state();
                                if let Err(error) = self.sync_manual_seek_into_detached_session(
                                    &projected_state,
                                    previous_position_seconds,
                                    target_position_seconds,
                                ) {
                                    Self::push_player_error(handle, error);
                                }
                                Self::push_player_success(
                                    handle,
                                    format!(
                                        "Applied a {offset_seconds} second seek via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                    ),
                                );
                            }
                            Err(error) => {
                                Self::push_player_error(
                                    handle,
                                    format!(
                                        "Playback seek through the attached {player_name} player failed: {error}"
                                    ),
                                );
                            }
                        }
                    } else {
                        Self::push_runtime_unavailable(
                            handle,
                            self.seek_unavailable_message(offset_seconds),
                        );
                    }
                }
                GuiRuntimeRequest::SetRoom(room) => {
                    self.request_room_join_runtime(handle, &mut projected_state, room);
                }
                GuiRuntimeRequest::ReturnToDefaultRoom => {
                    self.request_room_leave_runtime(handle, &mut projected_state);
                }
                GuiRuntimeRequest::SetLocalReady(ready) => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.set_local_ready(ready)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::SetReadyForUser { username, ready } => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.set_user_ready(username, ready)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::RequestControllerAuth { room, password } => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.request_controller_auth(room, password)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::QueuePlaylistEntry {
                    entry,
                    select_after_queue,
                } => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.queue_playlist_entry(entry, select_after_queue)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::SetPlaylistIndex(index) => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.set_playlist_index(index)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::DeletePlaylistIndex(index) => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.delete_playlist_index(index)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::UndoPlaylistChange => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.undo_playlist_change()
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::ShuffleRemainingPlaylist => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.shuffle_remaining_playlist()
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::ShuffleEntirePlaylist => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.shuffle_entire_playlist()
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::ReplacePlaylist {
                    files,
                    selected_index,
                } => {
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.replace_playlist(files, selected_index)
                    {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::TogglePlaybackPause,
                ) => {
                    self.refresh_player_state();
                    self.ensure_configured_player_attached();
                    if self.player.is_some() {
                        let target_paused = !projected_state.main_window.playback_paused;
                        let previous_paused = projected_state.main_window.playback_paused;
                        let (player_name, toggle_result) = {
                            let player = self.player.as_mut().expect("player should exist");
                            (player.name(), player.set_paused(target_paused))
                        };
                        let actions = match toggle_result {
                            Ok(()) => {
                                self.player_paused = Some(target_paused);
                                self.refresh_player_state();
                                if let Err(error) = self.sync_playback_pause_into_detached_session(
                                    &projected_state,
                                    previous_paused,
                                    target_paused,
                                ) {
                                    Self::push_player_error(handle, error);
                                }
                                vec![GuiShellAction::CompletePlaybackPauseToggle]
                            }
                            Err(error) => vec![
                                GuiShellAction::CancelPlaybackPauseToggle,
                                GuiShellAction::PushTransientNotification {
                                    level: GuiTransientNotificationLevel::Error,
                                    message: format!(
                                        "Playback pause toggle through the attached {player_name} player failed: {error}"
                                    ),
                                },
                            ],
                        };
                        Self::push_actions_and_project(handle, &mut projected_state, actions);
                    } else {
                        let actions = vec![
                            GuiShellAction::CancelPlaybackPauseToggle,
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: self.toggle_pause_unavailable_message(),
                            },
                        ];
                        Self::push_actions_and_project(handle, &mut projected_state, actions);
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::ConnectSavedServer,
                ) => {
                    self.complete_saved_server_connect_runtime(handle, &mut projected_state, true);
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::DisconnectSession,
                ) => {
                    self.complete_session_disconnect_runtime(handle, &mut projected_state);
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::ConnectPublicServer,
                ) => {
                    let selected_server = projected_state
                        .selected_public_server_index()
                        .and_then(|index| projected_state.public_servers.servers.get(index))
                        .map(|row| (row.label.clone(), row.address.clone()));
                    let replacement_transport_driver = selected_server
                        .as_ref()
                        .map(|(_label, address)| {
                            GuiTcpSessionTransportDriver::connect_from_host_arg(address).map(
                                |driver| Box::new(driver) as Box<dyn GuiSessionTransportDriver>,
                            )
                        })
                        .transpose();
                    let replacement_transport_driver = match replacement_transport_driver {
                        Ok(driver) => driver,
                        Err(error) => {
                            self.clear_pending_operation_with_runtime_error(
                                handle,
                                &mut projected_state,
                                format!(
                                    "Public server connect through the attached session runtime failed: {error}"
                                ),
                            );
                            continue;
                        }
                    };
                    if let Err(error) =
                        self.ensure_detached_client_core_chat_session(&projected_state)
                    {
                        self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            format!(
                                "Public server connect through the attached session runtime failed: {error}"
                            ),
                        );
                        continue;
                    }
                    let Some(session) = self.session.as_mut() else {
                        self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            "Public server connect could not bootstrap a detached client-core session runtime."
                                .to_owned(),
                        );
                        continue;
                    };
                    match session.connect_public_server(selected_server) {
                        Ok(()) => {
                            self.session_projects_to_shell = true;
                            if let Some(driver) = replacement_transport_driver {
                                if let Some(session_transport) = self.session_transport.as_ref() {
                                    session_transport.clear_protocol_lines();
                                }
                                self.session_transport_driver = Some(driver);
                            }
                            Self::push_actions_and_project(
                                handle,
                                &mut projected_state,
                                vec![GuiShellAction::CompleteSelectedPublicServerConnect],
                            )
                        }
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            format!(
                                "Public server connect through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::RefreshPublicServers(requested_servers),
                ) => {
                    let current_servers = projected_state
                        .public_servers
                        .servers
                        .iter()
                        .map(|row| (row.label.clone(), row.address.clone()))
                        .collect();
                    let refresh_result = if let Some(session) = self.session.as_mut() {
                        session.refresh_public_servers(current_servers)
                    } else if !requested_servers.is_empty() {
                        Ok(
                            GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                                requested_servers,
                            ),
                        )
                    } else {
                        Self::refresh_public_servers_without_session(current_servers)
                    };
                    match refresh_result {
                        Ok(servers) => Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![GuiShellAction::CompletePublicServerRefresh(servers)],
                        ),
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            format!(
                                "Public server refresh through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::SearchMissingMedia,
                ) => {
                    let directories = projected_state
                        .media_search
                        .directories
                        .iter()
                        .map(|row| row.path.clone())
                        .collect();
                    let search_result = if let Some(session) = self.session.as_mut() {
                        session.search_missing_media(directories)
                    } else {
                        self.search_missing_media_without_session(&projected_state, directories)
                    };
                    match search_result {
                        Ok(found_path) => {
                            let found_path =
                                found_path.and_then(|path| normalized_editable_text(&path));
                            self.ensure_configured_player_attached();
                            match found_path {
                                Some(path) if self.player.is_some() => {
                                    self.clear_pending_operation_runtime_state(
                                        handle,
                                        &mut projected_state,
                                    );
                                    self.open_media_files_through_attached_player(
                                        handle,
                                        vec![path],
                                    );
                                }
                                found_path => Self::push_actions_and_project(
                                    handle,
                                    &mut projected_state,
                                    vec![GuiShellAction::CompleteMissingMediaSearch(found_path)],
                                ),
                            }
                        }
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            format!(
                                "Missing-media search through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::SendChatMessage(message),
                ) => {
                    if let Some(session) = self.session.as_mut() {
                        match session.send_chat_message(message) {
                            Ok(()) => Self::push_actions_and_project(
                                handle,
                                &mut projected_state,
                                vec![GuiShellAction::CompleteLocalChatSend],
                            ),
                            Err(error) => self.clear_pending_operation_with_runtime_error(
                                handle,
                                &mut projected_state,
                                format!(
                                    "Chat sending through the attached session runtime failed: {error}"
                                ),
                            ),
                        }
                    } else {
                        self.clear_pending_operation_with_runtime_error(
                            handle,
                            &mut projected_state,
                            self.send_chat_unavailable_message(),
                        );
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::SaveConfiguration(settings),
                ) => {
                    let Some(path) = self.config_path.as_ref() else {
                        self.sync_player_from_lookup_and_settings(
                            &env_trimmed,
                            Some(&settings),
                            true,
                        );
                        Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![GuiShellAction::CompleteConfigurationSave(settings)],
                        );
                        continue;
                    };
                    match upsert_syncplay_ini_stored_client_settings_mvp_at_path(path, &settings) {
                        Ok(()) => {
                            self.sync_player_from_lookup_and_settings(
                                &env_trimmed,
                                Some(&settings),
                                true,
                            );
                            Self::push_actions_and_project(
                                handle,
                                &mut projected_state,
                                vec![GuiShellAction::CompleteConfigurationSave(settings)],
                            );
                        }
                        Err(error) => Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![
                                GuiShellAction::CancelConfigurationSave,
                                GuiShellAction::PushTransientNotification {
                                    level: GuiTransientNotificationLevel::Error,
                                    message: format!("Configuration save failed: {error}"),
                                },
                            ],
                        ),
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::ResetConfiguration(settings),
                ) => {
                    Self::push_actions_and_project(
                        handle,
                        &mut projected_state,
                        vec![GuiShellAction::CompleteConfigurationReset(settings)],
                    );
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::ReloadConfiguration(fallback_settings),
                ) => {
                    let Some(path) = self.config_path.as_ref() else {
                        Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![GuiShellAction::CompleteConfigurationReload(
                                fallback_settings,
                            )],
                        );
                        continue;
                    };
                    match load_syncplay_ini_stored_client_settings_mvp_from_path(path) {
                        Ok(Some(settings)) => {
                            self.sync_player_from_lookup_and_settings(
                                &env_trimmed,
                                Some(&settings),
                                true,
                            );
                            Self::push_actions_and_project(
                                handle,
                                &mut projected_state,
                                vec![GuiShellAction::CompleteConfigurationReload(settings)],
                            );
                        }
                        Ok(None) => {
                            self.sync_player_from_lookup_and_settings(
                                &env_trimmed,
                                Some(&fallback_settings),
                                true,
                            );
                            Self::push_actions_and_project(
                                handle,
                                &mut projected_state,
                                vec![GuiShellAction::CompleteConfigurationReload(
                                    fallback_settings,
                                )],
                            );
                        }
                        Err(error) => Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![
                                GuiShellAction::CancelConfigurationReload,
                                GuiShellAction::PushTransientNotification {
                                    level: GuiTransientNotificationLevel::Error,
                                    message: format!("Configuration reload failed: {error}"),
                                },
                            ],
                        ),
                    }
                }
                GuiRuntimeRequest::CompletePendingOperation(
                    GuiPendingCompletionRequest::ClearGuiData,
                ) => match self.clear_gui_data() {
                    Ok(()) => {
                        self.sync_player_from_lookup_and_settings(&env_trimmed, None, true);
                        Self::push_actions_and_project(
                            handle,
                            &mut projected_state,
                            vec![GuiShellAction::CompleteClearGuiData],
                        )
                    }
                    Err(error) => Self::push_actions_and_project(
                        handle,
                        &mut projected_state,
                        vec![
                            GuiShellAction::CancelClearGuiData,
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: format!("Clear GUI data failed: {error}"),
                            },
                        ],
                    ),
                },
                GuiRuntimeRequest::CancelPendingOperation(_kind) => {
                    Self::push_actions_and_project(
                        handle,
                        &mut projected_state,
                        vec![GuiShellAction::CancelPendingOperation],
                    );
                }
            }
            self.flush_session_transport_outbound(handle, &mut projected_state);
            self.pump_session_transport_driver(handle, &mut projected_state);
            self.drain_session_transport_inbound(handle, &mut projected_state);
            self.drain_session_runtime_actions(handle, &mut projected_state);
            self.drain_player_chat_input(handle, &mut projected_state);
        }
        self.sync_player_runtime_state(handle, &projected_state);
    }
}

#[derive(Default)]
struct GuiPreviewRuntimeBridge;

impl GuiPreviewRuntimeBridge {
    pub(crate) fn preview_open_media_file_actions(
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction> {
        if paths.is_empty() {
            return Vec::new();
        }

        let mut actions = vec![GuiShellAction::SwitchView(GuiShellView::MainWindow)];
        if load_into_shared_playlist {
            match GuiPersistedConfigRuntimeOwner::shared_playlist_open_dispatch_for_paths(paths) {
                Ok(dispatch) => {
                    actions.push(GuiShellAction::AnnounceSharedPlaylistLoaded(
                        dispatch.playlist_entries,
                    ));
                }
                Err(error) => {
                    actions.push(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    });
                    actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
                }
            }
            return actions;
        }

        let message = if paths.len() == 1 {
            format!("Media file selected: {}.", paths[0])
        } else {
            format!("Media files selected: {} entries.", paths.len())
        };
        actions.push(GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Info,
            message: message.clone(),
        });
        actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        actions
    }

    fn preview_seek_actions(offset_seconds: f64) -> Vec<GuiShellAction> {
        let message = format!("Seek requested: {offset_seconds} seconds.");
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn preview_offset_actions(command: &LocalOffsetCommand) -> Vec<GuiShellAction> {
        let message = format!("Offset requested: {}.", format_offset_command(command));
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn preview_pending_completion_actions(state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        GuiPendingCompletionRequest::from_state(state)
            .map(GuiPendingCompletionRequest::into_action)
            .into_iter()
            .collect()
    }

    fn preview_pending_cancel_actions(state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        state
            .pending_operation
            .as_ref()
            .map(|_| GuiShellAction::CancelPendingOperation)
            .into_iter()
            .collect()
    }
}

impl GuiNativeRuntimeBridge for GuiPreviewRuntimeBridge {
    fn shows_manual_pending_controls(&self) -> bool {
        true
    }

    fn actions_for_open_media_files(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    ) -> Vec<GuiShellAction> {
        Self::preview_open_media_file_actions(paths, load_into_shared_playlist)
    }

    fn actions_for_seek_offset(&mut self, offset_seconds: f64) -> Vec<GuiShellAction> {
        Self::preview_seek_actions(offset_seconds)
    }

    fn actions_for_undo_seek(&mut self) -> Vec<GuiShellAction> {
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Undo seek requested.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Undo seek requested.".to_owned()),
        ]
    }

    fn actions_for_set_offset(&mut self, command: LocalOffsetCommand) -> Vec<GuiShellAction> {
        Self::preview_offset_actions(&command)
    }

    fn actions_for_main_window_user_media_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        Self::preview_open_media_file_actions(vec![target], false)
    }

    fn actions_for_main_window_user_folder_open(
        &mut self,
        _state: &SyncplayGuiShellAppState,
        target: String,
    ) -> Vec<GuiShellAction> {
        let message = format!("Open containing folder requested: {target}.");
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    fn actions_for_pending_completion(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_completion_actions(state)
    }

    fn actions_for_pending_cancel(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::preview_pending_cancel_actions(state)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
enum GuiPendingCompletionRequest {
    SaveConfiguration(StoredClientSettingsMvp),
    ResetConfiguration(StoredClientSettingsMvp),
    ReloadConfiguration(StoredClientSettingsMvp),
    ClearGuiData,
    ConnectSavedServer,
    DisconnectSession,
    ConnectPublicServer,
    RefreshPublicServers(Vec<(String, String)>),
    SearchMissingMedia,
    TogglePlaybackPause,
    SendChatMessage(String),
}

impl GuiPendingCompletionRequest {
    fn from_state(state: &SyncplayGuiShellAppState) -> Option<Self> {
        let pending = state.pending_operation.as_ref()?;
        Some(match pending.kind {
            GuiPendingOperationKind::SaveConfiguration => {
                Self::SaveConfiguration(state.configuration.to_stored_settings())
            }
            GuiPendingOperationKind::ResetConfiguration => {
                Self::ResetConfiguration(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ReloadConfiguration => {
                Self::ReloadConfiguration(state.saved_configuration.clone())
            }
            GuiPendingOperationKind::ClearGuiData => Self::ClearGuiData,
            GuiPendingOperationKind::ConnectSavedServer => Self::ConnectSavedServer,
            GuiPendingOperationKind::DisconnectSession => Self::DisconnectSession,
            GuiPendingOperationKind::ConnectPublicServer => Self::ConnectPublicServer,
            GuiPendingOperationKind::RefreshPublicServers => Self::RefreshPublicServers(
                state
                    .public_servers
                    .servers
                    .iter()
                    .map(|row| (row.label.clone(), row.address.clone()))
                    .collect(),
            ),
            GuiPendingOperationKind::SearchMissingMedia => Self::SearchMissingMedia,
            GuiPendingOperationKind::TogglePlaybackPause => Self::TogglePlaybackPause,
            GuiPendingOperationKind::SendChatMessage => {
                Self::SendChatMessage(state.outgoing_chat_message.clone()?)
            }
        })
    }

    fn into_action(self) -> GuiShellAction {
        match self {
            Self::SaveConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationSave(settings)
            }
            Self::ResetConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationReset(settings)
            }
            Self::ReloadConfiguration(settings) => {
                GuiShellAction::CompleteConfigurationReload(settings)
            }
            Self::ClearGuiData => GuiShellAction::CompleteClearGuiData,
            Self::ConnectSavedServer => GuiShellAction::CompleteSavedServerConnect,
            Self::DisconnectSession => GuiShellAction::CompleteSessionDisconnect,
            Self::ConnectPublicServer => GuiShellAction::CompleteSelectedPublicServerConnect,
            Self::RefreshPublicServers(servers) => {
                GuiShellAction::CompletePublicServerRefresh(servers)
            }
            Self::SearchMissingMedia => GuiShellAction::CompleteMissingMediaSearch(None),
            Self::TogglePlaybackPause => GuiShellAction::CompletePlaybackPauseToggle,
            Self::SendChatMessage(_) => GuiShellAction::CompleteLocalChatSend,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuiPendingRoomChangeRequest {
    Join { requested_room: String },
    ReturnToDefault { previous_room: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiSharedPlaylistOpenDispatch {
    playlist_entries: Vec<String>,
    player_paths: Option<Vec<String>>,
    imported_from_file: bool,
}

#[allow(clippy::large_enum_variant)]
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
enum GuiRuntimeRequest {
    OpenMediaFiles {
        paths: Vec<String>,
        load_into_shared_playlist: bool,
    },
    OpenMainWindowUserMedia(String),
    OpenMainWindowUserContainingFolder(String),
    UndoSeek,
    SetOffset(LocalOffsetCommand),
    SetAutoplayEnabled(bool),
    SetAutoplayThreshold(usize),
    SetRoom(String),
    ReturnToDefaultRoom,
    SetLocalReady(bool),
    SetReadyForUser {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    QueuePlaylistEntry {
        entry: String,
        select_after_queue: bool,
    },
    SetPlaylistIndex(usize),
    DeletePlaylistIndex(usize),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    ReplacePlaylist {
        files: Vec<String>,
        selected_index: Option<usize>,
    },
    SeekOffset(f64),
    CompletePendingOperation(GuiPendingCompletionRequest),
    CancelPendingOperation(GuiPendingOperationKind),
}

impl GuiRuntimeRequest {
    fn preview_actions(&self) -> Vec<GuiShellAction> {
        match self {
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
            } => GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                paths.clone(),
                *load_into_shared_playlist,
            ),
            Self::OpenMainWindowUserMedia(target) => {
                GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                    vec![target.clone()],
                    false,
                )
            }
            Self::OpenMainWindowUserContainingFolder(target) => {
                let message = format!("Open containing folder requested: {target}.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::UndoSeek => vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Undo seek requested.".to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent("Undo seek requested.".to_owned()),
            ],
            Self::SetOffset(command) => {
                let message = format!("Offset requested: {}.", format_offset_command(command));
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetAutoplayEnabled(_)
            | Self::SetAutoplayThreshold(_)
            | Self::SetReadyForUser { .. }
            | Self::RequestControllerAuth { .. }
            | Self::QueuePlaylistEntry { .. }
            | Self::SetPlaylistIndex(_)
            | Self::DeletePlaylistIndex(_)
            | Self::UndoPlaylistChange
            | Self::ShuffleRemainingPlaylist
            | Self::ShuffleEntirePlaylist
            | Self::ReplacePlaylist { .. }
            | Self::SetRoom(_)
            | Self::ReturnToDefaultRoom => Vec::new(),
            Self::SeekOffset(offset_seconds) => {
                let message = format!("Seek requested: {offset_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetLocalReady(_) => Vec::new(),
            Self::CompletePendingOperation(request) => vec![request.clone().into_action()],
            Self::CancelPendingOperation(_) => vec![GuiShellAction::CancelPendingOperation],
        }
    }
}

impl GuiDialogControlKind {
    fn widget_kind(self) -> GuiWidgetKind {
        match self {
            Self::TextInput => GuiWidgetKind::TextInput,
            Self::PasswordInput => GuiWidgetKind::PasswordInput,
            Self::Checkbox => GuiWidgetKind::Checkbox,
            Self::Select => GuiWidgetKind::Select,
            Self::NumericInput => GuiWidgetKind::NumericInput,
            Self::ReadOnly => GuiWidgetKind::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiPendingOperationKind {
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
    fn label(self) -> &'static str {
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
struct GuiPendingOperationState {
    kind: GuiPendingOperationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiFocusedConfigurationControlState {
    section: &'static str,
    label: &'static str,
    kind: GuiDialogControlKind,
    activation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiPublicServerEditSessionState {
    editing_index: Option<usize>,
    label_buffer: String,
    address_buffer: String,
    is_dirty: bool,
    original_label: Option<String>,
    original_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiMainWindowUserEditSessionState {
    editing_index: usize,
    username_buffer: String,
    is_dirty: bool,
    original_username: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiTextEditSessionState {
    section: &'static str,
    label: &'static str,
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiPlaylistTextEditSessionState {
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiUrlEditSessionState {
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiControlledRoomCreateSessionState {
    room_buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiControllerAuthEditSessionState {
    room_name: String,
    password_buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiRoomHistoryEditSessionState {
    buffer: String,
    is_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiTransientNotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl GuiTransientNotificationLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiTransientNotification {
    level: GuiTransientNotificationLevel,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuiValidationIssue {
    scope: String,
    label: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct GuiValidationState {
    issues: Vec<GuiValidationIssue>,
    last_action_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiShellView {
    Configuration,
    MainWindow,
    MenusAndDialogs,
    PublicServers,
    MediaSearch,
}

impl GuiShellView {
    fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::MainWindow => "main-window",
            Self::MenusAndDialogs => "menus-and-dialogs",
            Self::PublicServers => "public-servers",
            Self::MediaSearch => "media-search",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
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
enum GuiShellModal {
    About,
    UpdateNotice,
    TlsCertificatePrompt,
}

impl GuiShellModal {
    fn label(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::UpdateNotice => "update-notice",
            Self::TlsCertificatePrompt => "tls-certificate-prompt",
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
enum GuiShellAction {
    SwitchView(GuiShellView),
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
    fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
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
                trusted_domains_label: optional_string_list_text(
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
                chat_input_font_family: settings.chat_input_font_family.clone(),
                chat_output_font_family: settings.chat_output_font_family.clone(),
            },
            osd: GuiOsdSection {
                show_osd: settings.show_osd.unwrap_or(false),
                show_duration_notification: settings.show_duration_notification.unwrap_or(false),
                show_same_room_osd: settings.show_same_room_osd.unwrap_or(false),
                show_osd_warnings: settings.show_osd_warnings.unwrap_or(false),
                show_noncontroller_osd: settings.show_noncontroller_osd.unwrap_or(false),
                show_different_room_osd: settings.show_different_room_osd.unwrap_or(false),
                show_contact_info: settings.show_contact_info.unwrap_or(false),
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
                compatibility_startup_entry_count: startup_entries.len(),
                ignored_startup_exception_count,
            },
        }
    }

    fn dialog_sections(&self) -> Vec<GuiDialogSection> {
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
                        value: optional_text(self.connection.room.as_deref()).to_owned(),
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
                        label: "Public Servers",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: self.connection.public_server_count.to_string(),
                    },
                    GuiDialogControl {
                        label: "Room History",
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
                        kind: GuiDialogControlKind::TextInput,
                        value: self.privacy.trusted_domains_label.clone(),
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
                        label: "Output Font",
                        kind: GuiDialogControlKind::TextInput,
                        value: optional_text(self.chat.chat_output_font_family.as_deref())
                            .to_owned(),
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
                        label: "Supported Languages",
                        kind: GuiDialogControlKind::ReadOnly,
                        value: SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY.to_owned(),
                    },
                ],
            },
        ]
    }
}

impl SyncplayGuiShellAppState {
    fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let mut state = Self {
            active_view: GuiShellView::Configuration,
            open_modal: None,
            selection: GuiSelectionState::default(),
            runtime_menu_action_overrides: Vec::new(),
            runtime_command_availability_override: GuiCommandAvailabilityRuntimeOverride::default(),
            commands: GuiCommandAvailabilityState::default(),
            pending_operation: None,
            outgoing_chat_message: None,
            new_main_window_user_draft: String::new(),
            new_playlist_entry_draft: String::new(),
            focused_configuration_control: None,
            public_server_edit_session: None,
            main_window_user_edit_session: None,
            text_edit_session: None,
            playlist_text_edit_session: None,
            playlist_url_edit_session: None,
            media_url_edit_session: None,
            controlled_room_create_session: None,
            controller_auth_edit_session: None,
            room_history_edit_session: None,
            update_check: GuiUpdateCheckState::default(),
            runtime_validation_issues: Vec::new(),
            notifications: Vec::new(),
            validation: GuiValidationState::default(),
            last_media_dialog_directory: None,
            playlist_undo_snapshot: None,
            playlist_shuffle_nonce: 0,
            saved_configuration: settings.clone(),
            configuration: FirstRunConfigurationDialogDraft::from_stored_settings(settings),
            main_window: MainWindowShellState::from_stored_settings(settings),
            menus: MenuDialogShellState::from_stored_settings(settings),
            public_servers: PublicServerBrowserShellState::from_stored_settings(settings),
            media_search: MediaSearchWorkflowShellState::from_stored_settings(settings),
        };
        state.default_selection_from_surfaces();
        state.apply_selection_to_surfaces();
        state.refresh_validation();
        state
    }

    fn saved_session_connect_target(&self) -> Option<GuiSavedSessionConnectTarget> {
        let raw_host = self
            .configuration
            .control_value("Connection", "Host")
            .unwrap_or_default()
            .trim();
        if raw_host.is_empty() {
            return None;
        }
        let (normalized_host, _) =
            parse_host_and_optional_port_from_host_arg_legacy_compatible(raw_host);
        let normalized_host = normalized_host.trim();
        if normalized_host.is_empty() {
            return None;
        }

        let raw_port = self
            .configuration
            .control_value("Connection", "Port")
            .unwrap_or_default()
            .trim();
        let port = if raw_port.is_empty() {
            self.configuration.to_stored_settings().port.unwrap_or(8999)
        } else {
            raw_port.parse::<u16>().ok().filter(|port| *port > 0)?
        };

        let mut settings = self.configuration.to_stored_settings();
        settings.host = Some(normalized_host.to_owned());
        settings.port = Some(port);
        settings.username = settings
            .username
            .take()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        settings.room = settings
            .room
            .take()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if settings.room.is_none()
            && let Some(room) = settings.room_list.as_ref().and_then(|rooms| {
                rooms.iter().find_map(|room| {
                    let trimmed = room.trim();
                    (!trimmed.is_empty()).then_some(trimmed.to_owned())
                })
            })
        {
            settings.room = Some(room);
        }
        let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let address = format!("{normalized_host}:{port}");
        Some(GuiSavedSessionConnectTarget {
            address,
            username: runtime_settings.settings.username.unwrap_or_default(),
            room: runtime_settings.settings.room.unwrap_or_default(),
            controlled_room_password_override: runtime_settings.controlled_room_password_override,
        })
    }

    fn saved_session_connect_button_label(&self) -> &'static str {
        if self.commands.can_disconnect_session {
            "Reconnect"
        } else {
            "Connect"
        }
    }

    fn apply_persisted_ui_state(&mut self, persisted_ui_state: &GuiPersistedUiState) {
        persisted_ui_state.apply_to_shell_state(self);
        self.refresh_validation();
        self.refresh_command_availability();
    }

    fn remember_media_dialog_directory(&mut self, path: &str) {
        let directory = Path::new(path)
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
            .map(|directory| directory.to_string_lossy().into_owned())
            .or_else(|| normalized_editable_text(path));
        self.last_media_dialog_directory = directory;
    }

    fn reset_to_first_run_state(&mut self, settings: StoredClientSettingsMvp) {
        *self = Self::from_stored_settings(&settings);
    }

    fn default_selection_from_surfaces(&mut self) {
        self.selection.selected_main_window_user =
            (!self.main_window.users.is_empty()).then_some(0);
        self.selection.selected_main_window_playlist = self
            .main_window
            .playlist
            .iter()
            .position(|row| row.is_selected)
            .or_else(|| (!self.main_window.playlist.is_empty()).then_some(0));
        self.selection.selected_menu_action =
            self.menus
                .sections
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    (!section.actions.is_empty()).then_some((section_index, 0))
                });
        self.selection.selected_media_search_directory =
            (!self.media_search.directories.is_empty()).then_some(0);
    }

    fn normalize_selection(&mut self) {
        if self
            .selection
            .selected_main_window_user
            .is_some_and(|index| index >= self.main_window.users.len())
        {
            self.selection.selected_main_window_user =
                (!self.main_window.users.is_empty()).then_some(0);
        }
        if self
            .selection
            .selected_main_window_playlist
            .is_some_and(|index| index >= self.main_window.playlist.len())
        {
            self.selection.selected_main_window_playlist =
                (!self.main_window.playlist.is_empty()).then_some(0);
        }
        if self
            .selection
            .selected_menu_action
            .is_some_and(|(section_index, action_index)| {
                self.menus
                    .sections
                    .get(section_index)
                    .is_none_or(|section| action_index >= section.actions.len())
            })
        {
            self.selection.selected_menu_action =
                self.menus
                    .sections
                    .iter()
                    .enumerate()
                    .find_map(|(section_index, section)| {
                        (!section.actions.is_empty()).then_some((section_index, 0))
                    });
        }
        if self
            .selection
            .selected_media_search_directory
            .is_some_and(|index| index >= self.media_search.directories.len())
        {
            self.selection.selected_media_search_directory =
                (!self.media_search.directories.is_empty()).then_some(0);
        }
    }

    fn normalize_selected_menu_action_after_runtime_update(&mut self) {
        let Some((selected_section_index, selected_action_index)) =
            self.selection.selected_menu_action
        else {
            return;
        };
        if self
            .menus
            .sections
            .get(selected_section_index)
            .and_then(|section| section.actions.get(selected_action_index))
            .is_some_and(|action| action.enabled)
        {
            return;
        }

        let replacement_in_section =
            self.menus
                .sections
                .get(selected_section_index)
                .and_then(|section| {
                    section
                        .actions
                        .iter()
                        .position(|action| action.enabled)
                        .map(|action_index| (selected_section_index, action_index))
                });
        self.selection.selected_menu_action = replacement_in_section.or_else(|| {
            self.menus
                .sections
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    section
                        .actions
                        .iter()
                        .position(|action| action.enabled)
                        .map(|action_index| (section_index, action_index))
                })
        });
    }

    fn set_menu_action_enabled(
        &mut self,
        section_title: &'static str,
        action_label: &'static str,
        enabled: bool,
    ) {
        let Some(action) = self
            .menus
            .sections
            .iter_mut()
            .find(|section| section.title == section_title)
            .and_then(|section| {
                section
                    .actions
                    .iter_mut()
                    .find(|action| action.label == action_label)
            })
        else {
            return;
        };
        action.enabled = enabled;
    }

    fn set_menu_action_selected(
        &mut self,
        section_title: &'static str,
        action_label: &'static str,
        selected: bool,
    ) {
        let Some(action) = self
            .menus
            .sections
            .iter_mut()
            .find(|section| section.title == section_title)
            .and_then(|section| {
                section
                    .actions
                    .iter_mut()
                    .find(|action| action.label == action_label)
            })
        else {
            return;
        };
        action.is_selected = selected;
    }

    fn set_runtime_menu_action_override(&mut self, action_override: MenuActionRuntimeOverride) {
        if let Some(existing) = self
            .runtime_menu_action_overrides
            .iter_mut()
            .find(|existing| {
                existing.section_title == action_override.section_title
                    && existing.action_label == action_override.action_label
            })
        {
            existing.enabled = action_override.enabled;
            return;
        }
        self.runtime_menu_action_overrides.push(action_override);
    }

    fn clear_runtime_menu_action_override(
        &mut self,
        section_title: &'static str,
        action_label: &'static str,
    ) {
        self.runtime_menu_action_overrides
            .retain(|action_override| {
                action_override.section_title != section_title
                    || action_override.action_label != action_label
            });
    }

    fn remember_runtime_menu_action_override(
        &mut self,
        baseline_menus: &MenuDialogShellState,
        action_override: &MenuActionRuntimeOverride,
    ) {
        let baseline_enabled = baseline_menus
            .sections
            .iter()
            .find(|section| section.title == action_override.section_title)
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == action_override.action_label)
            })
            .map(|action| action.enabled);
        let Some(baseline_enabled) = baseline_enabled else {
            return;
        };
        if action_override.enabled == baseline_enabled {
            self.clear_runtime_menu_action_override(
                action_override.section_title,
                action_override.action_label,
            );
            return;
        }
        self.set_runtime_menu_action_override(action_override.clone());
    }

    fn normalize_runtime_menu_action_overrides_for_settings(
        &mut self,
        settings: &StoredClientSettingsMvp,
    ) {
        let baseline_menus = MenuDialogShellState::from_stored_settings(settings);
        self.runtime_menu_action_overrides
            .retain(|action_override| {
                baseline_menus
                    .sections
                    .iter()
                    .find(|section| section.title == action_override.section_title)
                    .and_then(|section| {
                        section
                            .actions
                            .iter()
                            .find(|action| action.label == action_override.action_label)
                    })
                    .is_some_and(|action| action.enabled != action_override.enabled)
            });
    }

    fn command_availability_without_runtime_override(&self) -> GuiCommandAvailabilityState {
        let settings = self.configuration.to_stored_settings();
        let busy = self.pending_operation.is_some();
        GuiCommandAvailabilityState {
            can_save_configuration: !busy && self.validation.issues.is_empty(),
            can_reset_configuration: !busy && self.has_unsaved_configuration_changes(),
            can_reload_configuration: !busy,
            can_connect_saved_server: !busy && self.saved_session_connect_target().is_some(),
            can_disconnect_session: false,
            can_connect_public_server: !busy && self.public_servers.can_connect,
            can_refresh_public_servers: !busy && self.public_servers.can_refresh,
            can_search_missing_media: !busy && self.media_search.can_search_missing_media,
            can_toggle_pause: !busy && self.main_window.playback.can_toggle_pause,
            can_send_chat_message: !busy && settings.chat_input_enabled.unwrap_or(false),
        }
    }

    fn normalize_runtime_command_availability_override_for_current_state(&mut self) {
        let baseline = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override
            .normalize_for_baseline(&baseline);
    }

    fn sync_playback_menu_actions_from_runtime_state(&mut self, can_toggle_pause: bool) {
        let busy = self.pending_operation.is_some();
        let can_open_media_file = !busy && self.media_open_runtime_available();
        self.set_menu_action_enabled("File", "Open Media File", can_open_media_file);
        self.set_menu_action_enabled("Playback", "Play", can_toggle_pause);
        self.set_menu_action_enabled("Playback", "Pause", can_toggle_pause);
        self.set_menu_action_enabled("Playback", "Toggle Pause", can_toggle_pause);
        self.set_menu_action_enabled(
            "Playback",
            "Seek",
            !busy && self.main_window.playback.can_seek,
        );
        self.set_menu_action_enabled(
            "Playback",
            "Undo Seek",
            !busy && self.main_window.playback.can_undo_seek,
        );
        self.set_menu_action_enabled(
            "Playback",
            "Playlist Actions",
            !busy && self.main_window.playback.can_manage_playlist,
        );
        self.set_menu_action_enabled(
            "Advanced",
            "Set Offset",
            !busy && self.main_window.playback.can_set_offset,
        );
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
    }

    fn sync_dialog_menu_actions_from_runtime_state(&mut self) {
        let runtime_menu_action_overrides = self.runtime_menu_action_overrides.clone();
        for action_override in runtime_menu_action_overrides {
            self.set_menu_action_enabled(
                action_override.section_title,
                action_override.action_label,
                action_override.enabled,
            );
        }
        self.set_menu_action_enabled("Help", "About", self.menus.about_dialog_available);
    }

    fn open_newly_expected_modal_if_needed(
        &mut self,
        previous_tls_prompt_expected: bool,
        previous_update_notice_expected: bool,
    ) {
        if self.open_modal.is_some() {
            return;
        }
        if self.menus.tls_prompt_expected && !previous_tls_prompt_expected {
            self.open_modal = Some(GuiShellModal::TlsCertificatePrompt);
            return;
        }
        if self.menus.update_notice_expected && !previous_update_notice_expected {
            self.open_modal = Some(GuiShellModal::UpdateNotice);
        }
    }

    fn apply_selection_to_surfaces(&mut self) {
        for (index, user) in self.main_window.users.iter_mut().enumerate() {
            user.is_selected = self.selection.selected_main_window_user == Some(index);
        }
        for (index, item) in self.main_window.playlist.iter_mut().enumerate() {
            item.is_selected = self.selection.selected_main_window_playlist == Some(index);
        }
        for (section_index, section) in self.menus.sections.iter_mut().enumerate() {
            for (action_index, action) in section.actions.iter_mut().enumerate() {
                action.is_selected =
                    self.selection.selected_menu_action == Some((section_index, action_index));
            }
        }
        for (index, directory) in self.media_search.directories.iter_mut().enumerate() {
            directory.is_selected = self.selection.selected_media_search_directory == Some(index);
        }
    }

    fn move_selected_main_window_playlist(&mut self, delta: isize) -> bool {
        if !self.main_window.playback.can_manage_playlist {
            return self.record_action_error(
                "Playlist row movement is unavailable when shared playlist controls are disabled.",
            );
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No playlist row is currently selected.");
        };
        let Some(target_index) = index.checked_add_signed(delta) else {
            return self.record_action_error("The selected playlist row cannot move further.");
        };
        if target_index >= self.main_window.playlist.len() {
            return self.record_action_error("The selected playlist row cannot move further.");
        }

        let next_entries = {
            let mut entries = self.current_shared_playlist_entries();
            entries.swap(index, target_index);
            entries
        };
        self.remember_shared_playlist_undo_snapshot_if_changed(&next_entries);
        self.main_window.playlist.swap(index, target_index);
        self.selection.selected_main_window_playlist = Some(target_index);
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }

    fn remove_selected_main_window_playlist(&mut self) -> bool {
        if !self.main_window.playback.can_manage_playlist {
            return self.record_action_error(
                "Playlist row removal is unavailable when shared playlist controls are disabled.",
            );
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No playlist row is currently selected.");
        };
        if index >= self.main_window.playlist.len() {
            return self.record_action_error("No playlist row exists at the requested index.");
        }

        self.main_window.playlist.remove(index);
        self.selection.selected_main_window_playlist = if self.main_window.playlist.is_empty() {
            None
        } else if index >= self.main_window.playlist.len() {
            Some(self.main_window.playlist.len() - 1)
        } else {
            Some(index)
        };
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }

    fn add_main_window_user(&mut self, username: String) -> bool {
        let Some(username) = normalized_editable_text(&username) else {
            return self.record_action_error("Main-window user names must be non-empty.");
        };
        if self
            .main_window
            .users
            .iter()
            .any(|user| user.username.eq_ignore_ascii_case(&username))
        {
            return self.record_action_error("A main-window user with that name already exists.");
        }

        let room_name = self.main_window.room_name.clone();
        if !self
            .main_window
            .rooms
            .iter()
            .any(|room| room.room_name == room_name)
        {
            self.main_window.rooms.push(MainWindowRoomRow {
                room_name: room_name.clone(),
                is_controlled: room_name.starts_with('+'),
                has_named_users: true,
            });
        }
        self.main_window.users.push(MainWindowUserRow {
            username: username.clone(),
            room_name: room_name.clone(),
            is_self: false,
            is_ready: false,
            is_controller: false,
            has_file: false,
            file_name: None,
            file_name_label: "No file".to_owned(),
            file_size_label: String::new(),
            file_duration_label: String::new(),
            file_is_url: false,
            file_is_trusted: true,
            filename_differs: false,
            filesize_differs: false,
            fileduration_differs: false,
            is_selected: false,
        });
        if let Some(room) = self
            .main_window
            .rooms
            .iter_mut()
            .find(|room| room.room_name == room_name)
        {
            room.has_named_users = true;
        }
        self.selection.selected_main_window_user = Some(self.main_window.users.len() - 1);
        self.apply_selection_to_surfaces();
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("User joined: {username}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn rename_main_window_user_at_index(
        &mut self,
        index: usize,
        requested_username: String,
        empty_error_message: &'static str,
        missing_error_message: &'static str,
    ) -> Option<(String, String)> {
        let Some(username) = normalized_editable_text(&requested_username) else {
            self.record_action_error(empty_error_message);
            return None;
        };
        if self
            .main_window
            .users
            .iter()
            .enumerate()
            .any(|(other_index, user)| {
                other_index != index && user.username.eq_ignore_ascii_case(&username)
            })
        {
            self.record_action_error("A main-window user with that name already exists.");
            return None;
        }
        let Some(user) = self.main_window.users.get_mut(index) else {
            if self
                .main_window_user_edit_session
                .as_ref()
                .is_some_and(|session| session.editing_index == index)
            {
                self.main_window_user_edit_session = None;
            }
            self.record_action_error(missing_error_message);
            return None;
        };

        let previous_username = user.username.clone();
        user.username = username.clone();
        if user.is_self
            && !self
                .configuration
                .apply_text_value("Connection", "Username", &username)
        {
            user.username = previous_username;
            self.record_action_error(
                "The local user name could not be synchronized back into configuration state.",
            );
            return None;
        }

        Some((previous_username, username))
    }

    fn announce_main_window_user_joined(&mut self, username: String) -> bool {
        if !self.add_main_window_user(username) {
            return false;
        }
        let Some(user) = self.main_window.users.last() else {
            return self.record_action_error(
                "The announced main-window user could not be resolved after joining.",
            );
        };
        self.push_system_chat_message(format!("{} joined the room.", user.username));
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_selected_main_window_user_renamed(&mut self, username: String) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some((previous_username, renamed_username)) = self.rename_main_window_user_at_index(
            index,
            username,
            "Renamed main-window user names must be non-empty.",
            "The main-window user being renamed no longer exists.",
        ) else {
            return false;
        };

        self.main_window_user_edit_session = None;
        self.push_system_chat_message(format!(
            "{previous_username} is now known as {renamed_username}.",
        ));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("User renamed: {previous_username} -> {renamed_username}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_selected_main_window_user_left(&mut self) -> bool {
        let Some(index) = self.selection.selected_main_window_user else {
            return self.record_action_error("No main-window user is currently selected.");
        };
        let Some(user) = self.main_window.users.get(index) else {
            return self.record_action_error("No main-window user exists at the requested index.");
        };
        let username = user.username.clone();
        if !self.remove_selected_main_window_user() {
            return false;
        }
        self.push_system_chat_message(format!("{username} left the room."));
        self.clear_action_error_and_refresh();
        true
    }

    fn local_main_window_user_index(&self) -> Option<usize> {
        self.main_window.users.iter().position(|user| user.is_self)
    }

    fn current_joined_main_window_room_name(&self) -> Option<&str> {
        let room_name = self.main_window.room_name.trim();
        if room_name.is_empty() || room_name == "(no room joined)" {
            None
        } else {
            Some(room_name)
        }
    }

    fn main_window_local_can_control_current_room(&self) -> bool {
        if !self.main_window.controlled_room_active {
            return true;
        }
        let Some(room_name) = self.current_joined_main_window_room_name() else {
            return false;
        };
        self.main_window
            .users
            .iter()
            .any(|user| user.is_self && user.room_name == room_name && user.is_controller)
    }

    fn can_request_main_window_user_ready_change(&self, user: &MainWindowUserRow) -> bool {
        self.pending_operation.is_none()
            && self.commands.can_disconnect_session
            && self.main_window.playback.can_set_ready
            && self.main_window.playback.can_set_others_ready
            && !user.is_self
            && self
                .current_joined_main_window_room_name()
                .is_some_and(|room_name| user.room_name == room_name)
            && self.main_window_local_can_control_current_room()
    }

    fn controlled_room_create_default_room_name(&self) -> Option<String> {
        self.current_joined_main_window_room_name()
            .map(controlled_room_base_name_legacy_compatible)
            .and_then(|room_name| normalized_editable_text(&room_name))
    }

    fn begin_create_controlled_room_edit(&mut self) -> bool {
        let Some(room_name) = self.controlled_room_create_default_room_name() else {
            return self.record_action_error(
                "A joined room is required before creating a controlled room.",
            );
        };
        self.active_view = GuiShellView::MainWindow;
        self.controlled_room_create_session = Some(GuiControlledRoomCreateSessionState {
            room_buffer: room_name,
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    fn update_create_controlled_room_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.controlled_room_create_session.as_mut() else {
            return self
                .record_action_error("No controlled-room creation editor is currently active.");
        };
        session.room_buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_create_controlled_room_edit(&mut self) -> bool {
        if self.controlled_room_create_session.is_none() {
            return self
                .record_action_error("No controlled-room creation editor is currently active.");
        }
        self.controlled_room_create_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    fn begin_controller_auth_edit(&mut self) -> bool {
        let Some(room_name) = self
            .current_joined_main_window_room_name()
            .and_then(normalized_editable_text)
        else {
            return self.record_action_error(
                "A joined room is required before requesting controller access.",
            );
        };
        if !room_name.starts_with('+') {
            return self.record_action_error(
                "Controller access can only be requested while a controlled room is active.",
            );
        }
        self.active_view = GuiShellView::MainWindow;
        self.controller_auth_edit_session = Some(GuiControllerAuthEditSessionState {
            room_name,
            password_buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    fn update_controller_auth_password_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.controller_auth_edit_session.as_mut() else {
            return self.record_action_error("No controller-auth editor is currently active.");
        };
        session.password_buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_controller_auth_edit(&mut self) -> bool {
        if self.controller_auth_edit_session.is_none() {
            return self.record_action_error("No controller-auth editor is currently active.");
        }
        self.controller_auth_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    fn begin_playback_pause_state(&mut self, paused: bool) -> bool {
        if self.main_window.playback_paused == paused {
            return self.record_action_error(if paused {
                "Playback is already paused."
            } else {
                "Playback is already running."
            });
        }
        self.begin_playback_pause_toggle()
    }

    fn begin_playback_pause_toggle(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.main_window.playback.can_toggle_pause {
            return self.record_action_error(
                "Playback pause toggling is unavailable when pause controls are disabled.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::TogglePlaybackPause,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if self.main_window.playback_paused {
                "Playback resume requested.".to_owned()
            } else {
                "Playback pause requested.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn complete_playback_pause_toggle(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No playback toggle is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::TogglePlaybackPause {
            return self.record_action_error("The active GUI operation is not a playback toggle.");
        }

        self.pending_operation = None;
        self.announce_playback_pause_state(!self.main_window.playback_paused)
    }

    fn cancel_playback_pause_toggle(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No playback toggle is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::TogglePlaybackPause {
            return self.record_action_error("The active GUI operation is not a playback toggle.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Playback toggle canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_playback_pause_state(&mut self, paused: bool) -> bool {
        if !self.main_window.playback.can_toggle_pause {
            return self.record_action_error(
                "Playback pause state cannot change when pause controls are unavailable.",
            );
        }
        if self.main_window.playback_paused == paused {
            return self.record_action_error(if paused {
                "Playback is already paused."
            } else {
                "Playback is already running."
            });
        }

        self.main_window.playback_paused = paused;
        self.push_system_chat_message(if paused {
            "Playback paused.".to_owned()
        } else {
            "Playback resumed.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if paused {
                "Playback paused.".to_owned()
            } else {
                "Playback resumed.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_local_user_ready_state(&mut self, ready: bool) -> bool {
        if !self.main_window.playback.can_set_ready {
            return self.record_action_error(
                "Local readiness cannot change when ready controls are unavailable.",
            );
        }
        let Some(index) = self.local_main_window_user_index() else {
            return self
                .record_action_error("The local user row is missing from the main-window shell.");
        };
        let Some(user) = self.main_window.users.get_mut(index) else {
            return self
                .record_action_error("The local user row is missing from the main-window shell.");
        };
        if user.is_ready == ready {
            return self.record_action_error(if ready {
                "The local user is already marked ready."
            } else {
                "The local user is already marked not ready."
            });
        }

        user.is_ready = ready;
        self.push_system_chat_message(if ready {
            "You are now marked ready.".to_owned()
        } else {
            "You are now marked not ready.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if ready {
                "Local readiness updated: ready.".to_owned()
            } else {
                "Local readiness updated: not ready.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_autoplay_state(&mut self, active: bool) -> bool {
        if self.main_window.autoplay_active == active {
            return self.record_action_error(if active {
                "Autoplay is already active."
            } else {
                "Autoplay is already inactive."
            });
        }

        self.main_window.autoplay_active = active;
        self.push_system_chat_message(if active {
            "Autoplay enabled.".to_owned()
        } else {
            "Autoplay disabled.".to_owned()
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            if active {
                "Autoplay enabled.".to_owned()
            } else {
                "Autoplay disabled.".to_owned()
            },
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_autoplay_threshold(&mut self, threshold: usize) -> bool {
        if !(2..=99).contains(&threshold) {
            return self.record_action_error(
                "Autoplay minimum users must stay within the supported 2-99 range.",
            );
        }
        if self.main_window.autoplay_threshold == threshold {
            return self.record_action_error(
                "Autoplay minimum users is already set to the requested value.",
            );
        }

        self.main_window.autoplay_threshold = threshold;
        self.push_system_chat_message(format!("Autoplay minimum users set to {threshold}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Autoplay minimum users set to {threshold}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn toggle_main_window_playback_buttons(&mut self) -> bool {
        self.main_window.show_playback_buttons = !self.main_window.show_playback_buttons;
        self.set_menu_action_selected(
            "Window",
            "Playback Buttons",
            self.main_window.show_playback_buttons,
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn toggle_main_window_autoplay_controls(&mut self) -> bool {
        self.main_window.show_autoplay_controls = !self.main_window.show_autoplay_controls;
        self.set_menu_action_selected(
            "Window",
            "Autoplay",
            self.main_window.show_autoplay_controls,
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn shared_playlist_events_enabled(&self) -> bool {
        self.main_window.shared_playlist_enabled
    }

    fn media_open_runtime_available(&self) -> bool {
        self.main_window.playback.can_toggle_pause
            || self.main_window.playback.can_seek
            || self.main_window.playback.can_manage_playlist
    }

    fn shared_playlist_drop_target_available(&self) -> bool {
        self.shared_playlist_events_enabled() && self.main_window.playback.can_manage_playlist
    }

    fn ensure_shared_playlist_event_allowed(&mut self) -> bool {
        if self.shared_playlist_events_enabled() {
            true
        } else {
            self.record_action_error(
                "Shared playlist events are unavailable when shared playlists are disabled.",
            )
        }
    }

    fn normalize_shared_playlist_entries(entries: Vec<String>) -> Vec<String> {
        entries
            .into_iter()
            .filter_map(|entry| normalized_editable_text(&entry))
            .collect()
    }

    fn current_shared_playlist_entries(&self) -> Vec<String> {
        self.main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    fn remember_shared_playlist_undo_snapshot_if_changed(&mut self, next_entries: &[String]) {
        let current_entries = self.current_shared_playlist_entries();
        if current_entries != next_entries {
            self.playlist_undo_snapshot = Some(current_entries);
        }
    }

    fn shared_playlist_target_index_from_changed_entries(
        current_entries: &[String],
        current_index: Option<usize>,
        next_entries: &[String],
    ) -> usize {
        let Some(current_index) = current_index else {
            return 0;
        };
        if next_entries.len() <= 1 {
            return 0;
        }

        let mut index = current_index;
        while index <= current_entries.len() {
            if let Some(entry) = current_entries.get(index)
                && let Some(valid_index) =
                    next_entries.iter().position(|candidate| candidate == entry)
            {
                return valid_index;
            }
            index = index.saturating_add(1);
        }

        let mut index = current_index;
        while index > 0 {
            if let Some(entry) = current_entries.get(index)
                && let Some(valid_index) =
                    next_entries.iter().position(|candidate| candidate == entry)
            {
                return if valid_index < next_entries.len().saturating_sub(1) {
                    valid_index.saturating_add(1)
                } else {
                    valid_index
                };
            }
            index = index.saturating_sub(1);
        }
        0
    }

    fn apply_shared_playlist_entries(
        &mut self,
        entries: Vec<String>,
        selected_index: Option<usize>,
    ) {
        self.main_window.playlist = entries
            .iter()
            .map(|label| MainWindowPlaylistRow {
                label: label.clone(),
                is_selected: false,
            })
            .collect();
        self.selection.selected_main_window_playlist =
            selected_index.filter(|index| *index < self.main_window.playlist.len());
        self.apply_selection_to_surfaces();
    }

    fn next_shared_playlist_shuffle_seed(
        &mut self,
        entries: &[String],
        current_index: usize,
        shuffle_scope_remaining: bool,
    ) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(if shuffle_scope_remaining {
            &b"remaining"[..]
        } else {
            &b"entire"[..]
        });
        hasher.update((current_index as u64).to_le_bytes());
        hasher.update(self.playlist_shuffle_nonce.to_le_bytes());
        for entry in entries {
            hasher.update(entry.as_bytes());
            hasher.update([0]);
        }
        self.playlist_shuffle_nonce = self.playlist_shuffle_nonce.wrapping_add(1);

        let digest = hasher.finalize();
        let mut seed_bytes = [0u8; 8];
        seed_bytes.copy_from_slice(&digest[..8]);
        let seed = u64::from_le_bytes(seed_bytes);
        if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        }
    }

    fn selected_shared_playlist_entry(&self) -> Option<&str> {
        self.selection
            .selected_main_window_playlist
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.label.as_str())
    }

    fn replace_shared_playlist_entries_locally(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let entries = Self::normalize_shared_playlist_entries(entries);
        let current_entries = self.current_shared_playlist_entries();
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if entries.is_empty() {
            None
        } else {
            Some(
                Self::shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &entries,
                )
                .min(entries.len().saturating_sub(1)),
            )
        };
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        self.apply_shared_playlist_entries(entries.clone(), target_index);
        let message = if entries.is_empty() {
            "Shared playlist cleared.".to_owned()
        } else {
            format!("Shared playlist updated ({} entries).", entries.len())
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Success, message);
        self.clear_action_error_and_refresh();
        true
    }

    fn append_shared_playlist_entries_locally(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let entries = Self::normalize_shared_playlist_entries(entries);
        if entries.is_empty() {
            return self.record_action_error("Shared playlist entries must be non-empty.");
        }
        let mut playlist_entries = self.current_shared_playlist_entries();
        self.remember_shared_playlist_undo_snapshot_if_changed(
            &[playlist_entries.clone(), entries.clone()].concat(),
        );
        playlist_entries.extend(entries.iter().cloned());
        let selected_index = playlist_entries.len().checked_sub(1);
        self.apply_shared_playlist_entries(playlist_entries, selected_index);
        let message = if entries.len() == 1 {
            format!("Shared playlist entry added: {}.", entries[0])
        } else {
            format!("Shared playlist entries added: {} items.", entries.len())
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Info, message);
        self.clear_action_error_and_refresh();
        true
    }

    fn undo_shared_playlist_change(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let current_entries = self.current_shared_playlist_entries();
        let Some(previous_entries) = self.playlist_undo_snapshot.clone() else {
            return self.record_action_error("No shared playlist change is available to undo.");
        };
        if previous_entries == current_entries {
            return self.record_action_error("No shared playlist change is available to undo.");
        }
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if previous_entries.is_empty() {
            None
        } else {
            Some(
                Self::shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &previous_entries,
                )
                .min(previous_entries.len().saturating_sub(1)),
            )
        };
        self.playlist_undo_snapshot = Some(current_entries);
        self.apply_shared_playlist_entries(previous_entries, target_index);
        self.push_system_chat_message("Shared playlist undo requested.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Shared playlist undo requested.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn shuffle_remaining_shared_playlist(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(current_index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No shared playlist entry is currently selected.");
        };
        let current_entries = self.current_shared_playlist_entries();
        if current_index >= current_entries.len() {
            return self.record_action_error("No shared playlist entry is currently selected.");
        }
        let shuffle_start = current_index.saturating_add(1);
        if shuffle_start >= current_entries.len() {
            return self
                .record_action_error("No remaining shared playlist entries can be shuffled.");
        }
        let mut shuffled_entries = current_entries.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, true);
        shuffle_playlist_entries_in_place(&mut shuffled_entries[shuffle_start..], seed);
        if shuffled_entries == current_entries {
            return self
                .record_action_error("No remaining shared playlist entries can be shuffled.");
        }
        self.remember_shared_playlist_undo_snapshot_if_changed(&shuffled_entries);
        self.apply_shared_playlist_entries(shuffled_entries, Some(current_index));
        self.push_system_chat_message("Remaining shared playlist entries shuffled.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Remaining shared playlist entries shuffled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn shuffle_entire_shared_playlist(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let current_entries = self.current_shared_playlist_entries();
        if current_entries.is_empty() {
            return self.record_action_error("The shared playlist is currently empty.");
        }
        let current_index = self.selection.selected_main_window_playlist.unwrap_or(0);
        let mut shuffled_entries = current_entries.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, false);
        shuffle_playlist_entries_in_place(&mut shuffled_entries, seed);
        self.remember_shared_playlist_undo_snapshot_if_changed(&shuffled_entries);
        self.apply_shared_playlist_entries(shuffled_entries, Some(0));
        self.push_system_chat_message("Shared playlist shuffled.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Shared playlist shuffled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn begin_shared_playlist_text_edit(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        self.playlist_text_edit_session = Some(GuiPlaylistTextEditSessionState {
            buffer: playlist_entries_multiline_text(&self.current_shared_playlist_entries()),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    fn update_shared_playlist_text_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.playlist_text_edit_session.as_mut() else {
            return self.record_action_error("No shared playlist text editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = true;
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_shared_playlist_text_edit(&mut self) -> bool {
        if self.playlist_text_edit_session.is_none() {
            return self.record_action_error("No shared playlist text editor is currently active.");
        }
        self.playlist_text_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    fn begin_shared_playlist_url_edit(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        self.playlist_url_edit_session = Some(GuiUrlEditSessionState {
            buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    fn update_shared_playlist_url_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.playlist_url_edit_session.as_mut() else {
            return self.record_action_error("No shared playlist URL editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_shared_playlist_url_edit(&mut self) -> bool {
        if self.playlist_url_edit_session.is_none() {
            return self.record_action_error("No shared playlist URL editor is currently active.");
        }
        self.playlist_url_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    fn begin_media_url_edit(&mut self) -> bool {
        self.media_url_edit_session = Some(GuiUrlEditSessionState {
            buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    fn update_media_url_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.media_url_edit_session.as_mut() else {
            return self.record_action_error("No open-URL editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_media_url_edit(&mut self) -> bool {
        if self.media_url_edit_session.is_none() {
            return self.record_action_error("No open-URL editor is currently active.");
        }
        self.media_url_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    fn load_shared_playlist_from_file(
        &mut self,
        path: String,
        entries: Vec<String>,
        shuffled: bool,
    ) -> bool {
        self.remember_media_dialog_directory(&path);
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let mut entries = Self::normalize_shared_playlist_entries(entries);
        if shuffled && !entries.is_empty() {
            let seed = self.next_shared_playlist_shuffle_seed(&entries, 0, false);
            shuffle_playlist_entries_in_place(&mut entries, seed);
        }
        let target_index = (!entries.is_empty()).then_some(0);
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        self.apply_shared_playlist_entries(entries, target_index);
        let message = if shuffled {
            format!("Shared playlist loaded and shuffled from file: {path}.")
        } else {
            format!("Shared playlist loaded from file: {path}.")
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Success, message);
        self.clear_action_error_and_refresh();
        true
    }

    fn save_shared_playlist_to_file(&mut self, path: String) -> bool {
        self.remember_media_dialog_directory(&path);
        self.push_system_chat_message(format!("Shared playlist saved to file: {path}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Shared playlist saved to file: {path}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_shared_playlist_loaded(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let entries = Self::normalize_shared_playlist_entries(entries);
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        if entries.is_empty() {
            self.apply_shared_playlist_entries(Vec::new(), None);
            self.push_system_chat_message("Shared playlist cleared.".to_owned());
            self.push_transient_notification(
                GuiTransientNotificationLevel::Info,
                "Shared playlist cleared.".to_owned(),
            );
            self.clear_action_error_and_refresh();
            return true;
        }

        self.apply_shared_playlist_entries(entries, Some(0));
        self.push_system_chat_message(format!(
            "Shared playlist loaded ({} entries).",
            self.main_window.playlist.len()
        ));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!(
                "Shared playlist loaded: {} entries.",
                self.main_window.playlist.len()
            ),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_shared_playlist_entry_added(&mut self, entry: String) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(entry) = normalized_editable_text(&entry) else {
            return self.record_action_error("Shared playlist entries must be non-empty.");
        };
        let mut playlist_entries = self.current_shared_playlist_entries();
        playlist_entries.push(entry.clone());
        self.remember_shared_playlist_undo_snapshot_if_changed(&playlist_entries);
        let selected_index = playlist_entries.len().checked_sub(1);
        self.apply_shared_playlist_entries(playlist_entries, selected_index);
        self.push_system_chat_message(format!("Shared playlist entry added: {entry}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Shared playlist entry added: {entry}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_shared_playlist_selection_changed(&mut self, index: usize) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        if index >= self.main_window.playlist.len() {
            return self
                .record_action_error("No shared playlist entry exists at the requested index.");
        }
        self.selection.selected_main_window_playlist = Some(index);
        self.apply_selection_to_surfaces();
        let label = self.main_window.playlist[index].label.clone();
        self.push_system_chat_message(format!("Shared playlist selection changed: {label}.",));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Shared playlist selected: {label}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    fn announce_selected_shared_playlist_entry_removed(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No shared playlist entry is currently selected.");
        };
        let Some(entry) = self.main_window.playlist.get(index) else {
            return self
                .record_action_error("No shared playlist entry exists at the requested index.");
        };
        let label = entry.label.clone();
        let mut playlist_entries = self.current_shared_playlist_entries();
        playlist_entries.remove(index);
        self.remember_shared_playlist_undo_snapshot_if_changed(&playlist_entries);
        let next_selection = if playlist_entries.is_empty() {
            None
        } else if index >= playlist_entries.len() {
            Some(playlist_entries.len() - 1)
        } else {
            Some(index)
        };
        self.apply_shared_playlist_entries(playlist_entries, next_selection);
        self.push_system_chat_message(format!("Shared playlist entry removed: {label}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            format!("Shared playlist entry removed: {label}."),
        );
        self.clear_action_error_and_refresh();
        true
    }
}

pub fn run_syncplay_gui() {
    match run_syncplay_gui_semantic_cli_from_env() {
        Ok(Some(output)) => {
            println!("{output}");
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("syncplay-gui failed to run semantic scenario: {error}");
            std::process::exit(1);
        }
    }
    let (mut host, settings) = match gui_startup_host_and_settings() {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("syncplay-gui failed to configure startup runtime: {error}");
            std::process::exit(1);
        }
    };
    let persisted_ui_state = match load_gui_ui_state_from_lookup(&env_trimmed) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("syncplay-gui failed to load legacy GUI state: {error}");
            std::process::exit(1);
        }
    };
    let mut merged_settings = settings.clone();
    if let Some(persisted_ui_state) = persisted_ui_state.as_ref() {
        persisted_ui_state.merge_into_startup_settings(&mut merged_settings);
    }
    let startup_actions = gui_startup_actions_from_lookup(env_trimmed, &merged_settings);
    if let Err(error) = run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        persisted_ui_state.as_ref(),
        startup_actions,
        &mut host,
    ) {
        eprintln!("syncplay-gui failed to start: {error}");
        std::process::exit(1);
    }
}
