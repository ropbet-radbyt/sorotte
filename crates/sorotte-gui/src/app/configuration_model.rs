//! Configuration calculations shared by the shell and worker.
use super::shell_state::*;
use super::support::{
    chat_input_enabled, configured_room_name_text, normalized_editable_text,
    parse_trusted_domains_text,
};
use sorotte_client_app::app_boundary::language::normalized_runtime_language_tag;
use sorotte_client_app::app_boundary::state::*;
use sorotte_client_core::PrivacyMode;

pub(super) struct GuiConfigurationContext<'a> {
    pub(super) configuration: &'a FirstRunConfigurationDialogDraft,
    pub(super) public_servers: &'a PublicServerBrowserShellState,
    pub(super) media_search: &'a MediaSearchWorkflowShellState,
}

impl GuiConfigurationContext<'_> {
    pub(super) fn validation_issues(&self) -> Vec<GuiValidationIssue> {
        let mut issues = Vec::new();

        self.push_u16_validation_issue(
            &mut issues,
            SettingId::ConnectionPort,
            "must be a valid TCP port from 1 to 65535.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::PlaybackUnpauseAction,
            |value| parse_unpause_action_mode(value).is_some(),
            "must be a supported unpause action mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::PlaybackAutoplayMinUsers,
            |value| value == "app-default" || parse_autoplay_min_users_override(value).is_some(),
            "must be a supported autoplay threshold or 'app-default'.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::PrivacyFilename,
            |value| PrivacyMode::from_syncplay_name(value).is_some(),
            "must be a supported privacy mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::PrivacyFilesize,
            |value| PrivacyMode::from_syncplay_name(value).is_some(),
            "must be a supported privacy mode.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::PrivacyTrustedDomains,
            |value| parse_trusted_domains_text(value).is_some(),
            "must be a comma/semicolon-separated list or Python bracketed list.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingQuality,
            |value| StreamingQualityPreset::parse(value).is_some(),
            "must be a supported streaming quality preset.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingRecoveryPolicy,
            |value| StreamingRecoveryPolicy::parse(value).is_some(),
            "must be a supported recovery policy.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingRoomBufferingPolicy,
            |value| RoomBufferingPolicy::parse(value).is_some(),
            "must be a supported room buffering policy.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingStartSynchronization,
            |value| StartSynchronizationPolicy::parse(value).is_some(),
            "must be a supported start synchronization policy.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingStartTimeoutAction,
            |value| StartTimeoutAction::parse(value).is_some(),
            "must be continue, remain-paused, or ask-controller.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingMaximumCatchupRate,
            |value| {
                value
                    .parse::<f64>()
                    .is_ok_and(|value| value.is_finite() && (1.0..=1.25).contains(&value))
            },
            "must be between 1.0 and 1.25.",
        );
        for id in [
            SettingId::StreamingRoomQuorumPercent,
            SettingId::StreamingStartQuorumPercent,
        ] {
            self.push_parse_validation_issue(
                &mut issues,
                id,
                |value| {
                    value.parse::<f64>().is_ok_and(|value| {
                        value.is_finite() && (0.0..=100.0).contains(&value) && value > 0.0
                    })
                },
                "must be greater than 0 and no more than 100.",
            );
        }
        for id in [
            SettingId::StreamingMaximumHardSeeks,
            SettingId::StreamingRecoveryRetryBudget,
        ] {
            self.push_parse_validation_issue(
                &mut issues,
                id,
                |value| value.parse::<u64>().is_ok(),
                "must be a non-negative whole number.",
            );
        }
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::StreamingMemoryCacheMib,
            |value| value.parse::<u64>().is_ok_and(|value| value > 0),
            "must be a positive whole number.",
        );
        for id in [
            SettingId::StreamingBufferTargetSeconds,
            SettingId::StreamingReadAheadSeconds,
            SettingId::StreamingHardSeekThresholdSeconds,
            SettingId::StreamingStabilityIntervalSeconds,
            SettingId::StreamingRoomMaximumPauseSeconds,
            SettingId::StreamingStartTimeoutSeconds,
        ] {
            self.push_parse_validation_issue(
                &mut issues,
                id,
                |value| {
                    value
                        .parse::<f64>()
                        .is_ok_and(|value| value.is_finite() && value > 0.0)
                },
                "must be a finite positive number.",
            );
        }
        if self
            .configuration
            .control_value(SettingId::StreamingQuality)
            .is_some_and(|value| value.eq_ignore_ascii_case("custom"))
            && self
                .configuration
                .control_value(SettingId::StreamingCustomFormat)
                .and_then(normalized_editable_text)
                .is_none()
        {
            issues.push(GuiValidationIssue::for_setting(
                SettingId::StreamingCustomFormat,
                "must be set when the custom quality preset is selected.",
            ));
        }
        let buffer_target = self
            .configuration
            .control_value(SettingId::StreamingBufferTargetSeconds)
            .and_then(|value| value.parse::<f64>().ok());
        let read_ahead = self
            .configuration
            .control_value(SettingId::StreamingReadAheadSeconds)
            .and_then(|value| value.parse::<f64>().ok());
        if buffer_target
            .zip(read_ahead)
            .is_some_and(|(target, read_ahead)| read_ahead < target)
        {
            issues.push(GuiValidationIssue::for_setting(
                SettingId::StreamingReadAheadSeconds,
                "must be at least the buffer target.",
            ));
        }
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::ChatInputPosition,
            |value| matches!(value, "Top" | "Middle" | "Bottom"),
            "must be Top, Middle, or Bottom.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::ChatOutputMode,
            |value| matches!(value, "Chatroom" | "Scrolling"),
            "must be Chatroom or Scrolling.",
        );
        for id in [
            SettingId::SyncRewindThreshold,
            SettingId::SyncFastforwardThreshold,
            SettingId::SyncSlowdownThreshold,
            SettingId::StreamingRecoveryCooldownSeconds,
            SettingId::MediaLibraryFirstFileTimeout,
            SettingId::MediaLibrarySearchTimeout,
            SettingId::MediaLibraryDoubleCheckInterval,
            SettingId::MediaLibraryWarningThreshold,
        ] {
            self.push_nonnegative_f64_validation_issue(
                &mut issues,
                id,
                "must be a finite non-negative number.",
            );
        }
        for (id, message) in [
            (SettingId::ChatInputFontSize, "must be a positive integer."),
            (SettingId::ChatOutputFontSize, "must be a positive integer."),
        ] {
            self.push_positive_i64_validation_issue(&mut issues, id, message);
        }
        for id in [
            SettingId::ChatInputFontWeight,
            SettingId::ChatOutputFontWeight,
            SettingId::ChatTopMargin,
            SettingId::ChatLeftMargin,
            SettingId::ChatBottomMargin,
            SettingId::ChatOsdMargin,
            SettingId::OsdNotificationTimeout,
            SettingId::OsdAlertTimeout,
            SettingId::OsdChatTimeout,
        ] {
            self.push_nonnegative_i64_validation_issue(
                &mut issues,
                id,
                "must be a non-negative integer.",
            );
        }
        self.push_positive_i64_validation_issue(
            &mut issues,
            SettingId::ChatMaxLines,
            "must be a positive integer.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::GeneralLanguage,
            |value| normalized_runtime_language_tag(value).is_some(),
            "must be one of the supported language tags.",
        );
        self.push_parse_validation_issue(
            &mut issues,
            SettingId::GeneralUpdateChannel,
            |value| matches!(value.to_ascii_lowercase().as_str(), "stable" | "dev"),
            "must be stable or dev.",
        );

        let mut seen_directories = std::collections::BTreeSet::new();
        for directory in &self.media_search.directories {
            if !seen_directories.insert(directory.path.clone()) {
                issues.push(GuiValidationIssue::for_setting(
                    SettingId::MediaLibraryDirectories,
                    "contains duplicate search directories.",
                ));
                break;
            }
        }

        for row in &self.public_servers.servers {
            let (host, _) = parse_host_and_optional_port_from_host_arg(&row.address);
            if host.trim().is_empty() {
                issues.push(GuiValidationIssue::external(
                    "Public Servers",
                    "Address",
                    format!("'{}' is not a valid server address.", row.address),
                ));
            }
        }

        issues
    }

    pub(super) fn push_parse_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        id: SettingId,
        is_valid: impl FnOnce(&str) -> bool,
        message: &'static str,
    ) {
        let Some(value) = self.configuration.control_value(id) else {
            return;
        };
        let Some(normalized) = normalized_editable_text(value) else {
            return;
        };
        if !is_valid(&normalized) {
            issues.push(GuiValidationIssue::for_setting(id, message));
        }
    }

    pub(super) fn push_u16_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        id: SettingId,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            id,
            |value| value.parse::<u16>().is_ok_and(|parsed| parsed > 0),
            message,
        );
    }

    pub(super) fn push_positive_i64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        id: SettingId,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            id,
            |value| value.parse::<i64>().is_ok_and(|parsed| parsed > 0),
            message,
        );
    }

    pub(super) fn push_nonnegative_i64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        id: SettingId,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            id,
            |value| value.parse::<i64>().is_ok_and(|parsed| parsed >= 0),
            message,
        );
    }

    pub(super) fn push_nonnegative_f64_validation_issue(
        &self,
        issues: &mut Vec<GuiValidationIssue>,
        id: SettingId,
        message: &'static str,
    ) {
        self.push_parse_validation_issue(
            issues,
            id,
            |value| {
                value
                    .parse::<f64>()
                    .is_ok_and(|parsed| parsed.is_finite() && parsed >= 0.0)
            },
            message,
        );
    }
}
pub(super) fn saved_session_connect_target(
    configuration: &FirstRunConfigurationDialogDraft,
) -> Option<GuiSavedSessionConnectTarget> {
    let raw_host = configuration
        .control_value(SettingId::ConnectionHost)
        .unwrap_or_default()
        .trim();
    if raw_host.is_empty() {
        return None;
    }
    let (normalized_host, _) = parse_host_and_optional_port_from_host_arg(raw_host);
    let normalized_host = normalized_host.trim();
    if normalized_host.is_empty() {
        return None;
    }

    let raw_port = configuration
        .control_value(SettingId::ConnectionPort)
        .unwrap_or_default()
        .trim();
    let port = if raw_port.is_empty() {
        configuration.to_stored_settings().port.unwrap_or(8999)
    } else {
        raw_port.parse::<u16>().ok().filter(|port| *port > 0)?
    };

    let mut settings = configuration.to_stored_settings();
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
        .and_then(|value| configured_room_name_text(&value));
    if settings.room.is_none()
        && let Some(room) = settings.room_list.as_ref().and_then(|rooms| {
            rooms
                .iter()
                .find_map(|room| (!room.is_empty()).then_some(room.to_owned()))
        })
    {
        settings.room = Some(room);
    }
    let runtime_settings = stored_client_settings_runtime_snapshot(&settings);
    let address = format!("{normalized_host}:{port}");
    Some(GuiSavedSessionConnectTarget {
        address,
        username: runtime_settings
            .config
            .connection
            .username
            .map(|username| username.into_inner())
            .unwrap_or_default(),
        room: runtime_settings
            .config
            .connection
            .room
            .map(|room| room.into_inner())
            .unwrap_or_default(),
        controlled_room_password_override: runtime_settings
            .config
            .connection
            .controlled_room_password,
    })
}

pub(super) fn connect_blocked_by_player_setup_issue(
    configuration: &FirstRunConfigurationDialogDraft,
    player_setup_issue: &Option<GuiPlayerSetupIssue>,
) -> bool {
    configuration.launch_mode == super::GuiLaunchMode::FirstRun
        && player_setup_issue.as_ref().is_some_and(|issue| {
            !matches!(
                issue.kind,
                GuiPlayerSetupIssueKind::PlayerSettingsDegraded
                    | GuiPlayerSetupIssueKind::BridgeDegraded
            )
        })
}

pub(super) fn chat_send_unavailable_reason_from_settings(
    pending_operation: &Option<GuiPendingOperationState>,
    settings: &StoredClientSettings,
    session_runtime_available: bool,
) -> Option<String> {
    if !chat_input_enabled(settings) {
        return Some("Chat input is disabled in Chat settings.".to_owned());
    }
    if pending_operation.is_some() {
        return Some(
            "Chat input is unavailable while another GUI operation is in progress.".to_owned(),
        );
    }
    if !session_runtime_available {
        return Some(
            "Chat input is unavailable because no session runtime is connected.".to_owned(),
        );
    }
    None
}

pub(in crate::app) fn connect_once_runtime_settings(
    configuration: &FirstRunConfigurationDialogDraft,
    saved_configuration: &StoredClientSettings,
) -> StoredClientSettings {
    let draft = configuration.to_stored_settings();
    let mut settings = saved_configuration.clone();
    settings.host = draft.host;
    settings.port = draft.port;
    settings.username = draft.username;
    settings.room = draft.room;
    if !matches!(&configuration.server_password, SecretDraft::Unchanged) {
        settings.server_password = draft.server_password;
    }
    settings
}

pub(in crate::app) fn submitted_saved_server_connect_settings(
    configuration: &FirstRunConfigurationDialogDraft,
    saved_configuration: &StoredClientSettings,
    intent: GuiSavedServerConnectIntent,
) -> StoredClientSettings {
    match intent {
        GuiSavedServerConnectIntent::ConnectOnce => {
            connect_once_runtime_settings(configuration, saved_configuration)
        }
        GuiSavedServerConnectIntent::SaveAndConnect => configuration.to_stored_settings(),
    }
}

pub(super) struct GuiCommandAvailabilityContext<'a> {
    pub(super) configuration: &'a FirstRunConfigurationDialogDraft,
    pub(super) saved_configuration: &'a StoredClientSettings,
    pub(super) pending_config_storage_target: &'a Option<GuiConfigStorageChangeTarget>,
    pub(super) pending_operation: &'a Option<GuiPendingOperationState>,
    pub(super) player_setup_issue: &'a Option<GuiPlayerSetupIssue>,
    pub(super) validation: &'a GuiValidationState,
    pub(super) main_window: &'a MainWindowShellState,
    pub(super) public_servers: &'a PublicServerBrowserShellState,
    pub(super) media_search: &'a MediaSearchWorkflowShellState,
}
impl GuiCommandAvailabilityContext<'_> {
    pub(super) fn command_availability_without_runtime_override(
        &self,
    ) -> GuiCommandAvailabilityState {
        let settings = self.configuration.to_stored_settings();
        let busy = self.pending_operation.is_some();
        let chat_unavailable_reason =
            self.chat_send_unavailable_reason_from_settings(&settings, true);
        GuiCommandAvailabilityState {
            can_save_configuration: !busy
                && self.validation.issues.is_empty()
                && self.has_unsaved_configuration_changes(),
            can_reset_configuration: !busy && self.has_unsaved_configuration_changes(),
            can_reload_configuration: !busy,
            can_connect_saved_server: !busy
                && self.saved_session_connect_target().is_some()
                && !self.connect_blocked_by_player_setup_issue(),
            can_disconnect_session: false,
            can_connect_public_server: !busy && self.public_servers.can_connect,
            can_refresh_public_servers: !busy && self.public_servers.can_refresh,
            can_search_missing_media: !busy && self.media_search.can_search_missing_media,
            can_toggle_pause: !busy && self.main_window.playback.can_toggle_pause,
            can_send_chat_message: chat_unavailable_reason.is_none(),
            chat_unavailable_reason,
        }
    }

    fn has_unsaved_configuration_changes(&self) -> bool {
        self.pending_config_storage_target.is_some()
            || self
                .configuration
                .has_unsaved_changes_against(self.saved_configuration)
    }
    fn saved_session_connect_target(&self) -> Option<GuiSavedSessionConnectTarget> {
        saved_session_connect_target(self.configuration)
    }
    fn connect_blocked_by_player_setup_issue(&self) -> bool {
        connect_blocked_by_player_setup_issue(self.configuration, self.player_setup_issue)
    }
    fn chat_send_unavailable_reason_from_settings(
        &self,
        settings: &StoredClientSettings,
        connected: bool,
    ) -> Option<String> {
        chat_send_unavailable_reason_from_settings(self.pending_operation, settings, connected)
    }
}

pub(super) fn apply_persisted_settings_patch(
    saved_configuration: &mut StoredClientSettings,
    configuration: &mut FirstRunConfigurationDialogDraft,
    plugin_enablement: &mut GuiPluginEnablementState,
    media_match: &mut GuiMediaMatchState,
    plex: &mut GuiPlexState,
    patch: GuiPersistedSettingsPatch,
) -> bool {
    let mut refresh_sources = false;
    patch.apply_to(&mut (*saved_configuration));
    patch.apply_to(&mut configuration.settings);

    match patch {
        GuiPersistedSettingsPatch::PluginEnabled { plugin, enabled } => {
            (*plugin_enablement).set_enabled_for(plugin, enabled);
            refresh_sources = true;
        }
        GuiPersistedSettingsPatch::MediaMatchFingerprintingEnabled(enabled) => {
            media_match.settings.fingerprinting_enabled = enabled;
            refresh_sources = true;
        }
        GuiPersistedSettingsPatch::MediaMatchBackgroundWarmupEnabled(enabled) => {
            media_match.settings.background_warmup_enabled = enabled;
        }
        GuiPersistedSettingsPatch::MediaMatchWireSharingEnabled(enabled) => {
            media_match.settings.wire_sharing_enabled = enabled;
        }
        GuiPersistedSettingsPatch::MediaMatchRuntimeToleranceEnabled(enabled) => {
            media_match.settings.runtime_tolerance_enabled = enabled;
        }
        GuiPersistedSettingsPatch::MediaMatchAutoplayPolicy(policy) => {
            media_match.settings.autoplay_policy = policy;
        }
        GuiPersistedSettingsPatch::PlexAuthenticated {
            clear_selected_server,
            ..
        } => {
            plex.authenticated = true;
            if clear_selected_server {
                plex.selected_server_id = None;
                plex.selected_server_url = None;
            }
        }
        GuiPersistedSettingsPatch::PlexServerSelected {
            machine_identifier,
            uri,
            ..
        } => {
            plex.selected_server_id = Some(machine_identifier);
            plex.selected_server_url = Some(uri);
        }
        GuiPersistedSettingsPatch::PlexSyncEnabled(enabled) => {
            plex.enabled = enabled;
        }
        GuiPersistedSettingsPatch::PlexStreamingEnabled(enabled) => {
            plex.streaming_enabled = enabled;
        }
        GuiPersistedSettingsPatch::PlexDisconnected => {
            plex.enabled = false;
            plex.streaming_enabled = false;
            plex.authenticated = false;
            plex.selected_server_id = None;
            plex.selected_server_url = None;
        }
    }

    refresh_sources
}

pub(super) fn preserves_runtime_dialog_expectations(
    menus: &MenuDialogShellState,
    previous_settings: &StoredClientSettings,
) -> (bool, bool) {
    let previous_baseline = MenuDialogShellState::from_stored_settings(previous_settings);
    (
        menus.tls_prompt_expected != previous_baseline.tls_prompt_expected,
        menus.update_notice_expected != previous_baseline.update_notice_expected,
    )
}

pub(super) fn preserves_runtime_public_server_surface(
    public_servers: &PublicServerBrowserShellState,
    previous_settings: &StoredClientSettings,
) -> bool {
    let previous_baseline = PublicServerBrowserShellState::from_stored_settings(previous_settings);
    PublicServerBrowserRuntimeFlags::from_shell_state(public_servers)
        != PublicServerBrowserRuntimeFlags::from_shell_state(&previous_baseline)
}

pub(super) fn preserves_runtime_media_search_surface(
    media_search: &MediaSearchWorkflowShellState,
    previous_settings: &StoredClientSettings,
) -> bool {
    let previous_baseline = MediaSearchWorkflowShellState::from_stored_settings(previous_settings);
    MediaSearchWorkflowRuntimeFlags::from_shell_state(media_search)
        != MediaSearchWorkflowRuntimeFlags::from_shell_state(&previous_baseline)
}

pub(super) fn normalize_runtime_menu_action_overrides_for_settings(
    overrides: &mut Vec<MenuActionRuntimeOverride>,
    settings: &StoredClientSettings,
) {
    let baseline_menus = MenuDialogShellState::from_stored_settings(settings);
    overrides.retain(|action_override| {
        baseline_menus
            .action(action_override.id)
            .is_some_and(|action| action.enabled != action_override.enabled)
    });
}

pub(super) fn set_selected_public_server_index(
    servers: &mut PublicServerBrowserShellState,
    selected: Option<usize>,
) {
    for (index, row) in servers.servers.iter_mut().enumerate() {
        row.is_selected = selected == Some(index);
    }
}
pub(super) fn restore_selected_public_server_address(
    servers: &mut PublicServerBrowserShellState,
    address: Option<&str>,
) {
    let Some(index) = address.and_then(|address| {
        servers
            .servers
            .iter()
            .position(|row| row.address == address)
    }) else {
        return;
    };
    set_selected_public_server_index(servers, Some(index));
}
pub(super) fn apply_public_server_selection(
    servers: &mut PublicServerBrowserShellState,
    configuration: &mut FirstRunConfigurationDialogDraft,
    index: usize,
) -> Result<(), &'static str> {
    let Some(row) = servers.servers.get(index) else {
        return Err("No public server exists at the requested index.");
    };
    let (host, port) = parse_host_and_optional_port_from_host_arg(&row.address);
    set_selected_public_server_index(servers, Some(index));
    let _ = configuration.apply_text_value(SettingId::ConnectionHost, &host);
    let _ = configuration.apply_text_value(
        SettingId::ConnectionPort,
        &port.map_or_else(String::new, |value| value.to_string()),
    );
    Ok(())
}
pub(super) fn normalize_public_servers(servers: Vec<(String, String)>) -> Vec<(String, String)> {
    servers
        .into_iter()
        .filter_map(|(label, address)| {
            let label = normalized_editable_text(&label)?;
            let address = normalized_editable_text(&address)?;
            let (host, _) = parse_host_and_optional_port_from_host_arg(&address);
            (!host.trim().is_empty()).then_some((label, address))
        })
        .collect()
}
