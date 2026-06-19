use std::path::Path;

use sorotte_client_app::app_boundary::state::{
    StoredClientSettingsMvp, parse_host_and_optional_port_from_host_arg_legacy_compatible,
    stored_client_settings_runtime_snapshot_legacy_compatible,
};

use super::shell_state::{
    FirstRunConfigurationDialogDraft, GuiCommandAvailabilityRuntimeOverride,
    GuiCommandAvailabilityState, GuiConfigStorageRuntimeSnapshot, GuiConfigurationTab,
    GuiMediaMatchState, GuiPlayerSetupIssueKind, GuiPlexState, GuiPluginSelection,
    GuiSavedSessionConnectTarget, GuiSelectionState, GuiShellModal, GuiShellView,
    GuiValidationState, MainWindowShellState, MediaSearchWorkflowShellState,
    MenuActionRuntimeOverride, MenuDialogShellState, PublicServerBrowserShellState,
    SorotteGuiShellAppState,
};
use super::support::{
    configured_room_name_text, legacy_chat_input_enabled, normalized_editable_text,
};
use super::ui_state::{GuiPersistedUiState, GuiUpdateCheckState};

impl SorotteGuiShellAppState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(settings);
        let mut shell_settings = settings.clone();
        shell_settings.room = runtime_settings.settings.room.clone().map(|room| {
            runtime_settings
                .controlled_room_password_override
                .as_ref()
                .map_or(room.clone(), |password| format!("{room}:{password}"))
        });
        let mut state = Self {
            active_view: GuiShellView::Setup,
            selected_configuration_tab: GuiConfigurationTab::Connection,
            selected_plugin: GuiPluginSelection::default(),
            open_modal: None,
            selection: GuiSelectionState::default(),
            main_window_playlist_selection_is_local: false,
            runtime_menu_action_overrides: Vec::new(),
            runtime_command_availability_override: GuiCommandAvailabilityRuntimeOverride::default(),
            config_storage: GuiConfigStorageRuntimeSnapshot::default(),
            commands: GuiCommandAvailabilityState::default(),
            pending_operation: None,
            pending_config_storage_target: None,
            pending_local_ready_target: None,
            pending_saved_server_connect_saves_configuration: false,
            outgoing_chat_message: None,
            main_window_room_change_expanded: false,
            new_main_window_user_draft: String::new(),
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
            media_index_status: Default::default(),
            player_setup_issue: None,
            stream_helper: Default::default(),
            stream_helper_remediation: Default::default(),
            media_match: GuiMediaMatchState::from_stored_settings(&shell_settings),
            media_match_remediation: Default::default(),
            plex: GuiPlexState::from_stored_settings(&shell_settings),
            saved_configuration: shell_settings.clone(),
            configuration: FirstRunConfigurationDialogDraft::from_stored_settings(&shell_settings),
            main_window: MainWindowShellState::from_stored_settings(&shell_settings),
            menus: MenuDialogShellState::from_stored_settings(&shell_settings),
            public_servers: PublicServerBrowserShellState::from_stored_settings(&shell_settings),
            media_search: MediaSearchWorkflowShellState::from_stored_settings(&shell_settings),
        };
        state.default_selection_from_surfaces();
        state.apply_selection_to_surfaces();
        state.refresh_validation();
        state
    }

    pub(super) fn saved_session_connect_target(&self) -> Option<GuiSavedSessionConnectTarget> {
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
        let runtime_settings = stored_client_settings_runtime_snapshot_legacy_compatible(&settings);
        let address = format!("{normalized_host}:{port}");
        Some(GuiSavedSessionConnectTarget {
            address,
            username: runtime_settings.settings.username.unwrap_or_default(),
            room: runtime_settings.settings.room.unwrap_or_default(),
            controlled_room_password_override: runtime_settings.controlled_room_password_override,
        })
    }

    pub(super) fn saved_session_connect_button_label(&self) -> &'static str {
        if self.commands.can_disconnect_session {
            "Reconnect"
        } else {
            "Connect"
        }
    }

    pub(super) fn connect_blocked_by_player_setup_issue(&self) -> bool {
        self.configuration.launch_mode == super::GuiLaunchMode::FirstRun
            && self.player_setup_issue.is_some()
    }

    pub(super) fn player_setup_connect_block_message(&self) -> Option<String> {
        if !self.connect_blocked_by_player_setup_issue() {
            return None;
        }
        Some(
            "Set up mpv before connecting. Use Auto-detect, Choose mpv.exe, or Retry mpv after updating Player Path."
                .to_owned(),
        )
    }

    pub(super) fn player_setup_issue_title(&self) -> Option<&'static str> {
        self.player_setup_issue
            .as_ref()
            .map(|issue| match issue.kind {
                GuiPlayerSetupIssueKind::NotConfigured => "mpv setup required",
                GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer => "Unsupported player",
                GuiPlayerSetupIssueKind::MissingBinary => "Configured mpv not found",
                GuiPlayerSetupIssueKind::LaunchFailed => "mpv failed to launch",
                GuiPlayerSetupIssueKind::IpcAttachFailed => "mpv did not respond",
                GuiPlayerSetupIssueKind::ExitedAfterLaunch => "mpv closed unexpectedly",
            })
    }

    pub(super) fn player_setup_issue_summary(&self) -> Option<&'static str> {
        self.player_setup_issue
            .as_ref()
            .map(|issue| match issue.kind {
                GuiPlayerSetupIssueKind::NotConfigured => {
                    "Sorotte needs mpv before it can play media."
                }
                GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer => {
                    "The GUI currently supports mpv startup only."
                }
                GuiPlayerSetupIssueKind::MissingBinary => {
                    "The configured Player Path does not point to an mpv binary."
                }
                GuiPlayerSetupIssueKind::LaunchFailed => {
                    "Sorotte could not start mpv from the current Player Path."
                }
                GuiPlayerSetupIssueKind::IpcAttachFailed => {
                    "mpv started or was targeted, but Sorotte could not attach to its JSON IPC."
                }
                GuiPlayerSetupIssueKind::ExitedAfterLaunch => {
                    "mpv exited after it had already been launched."
                }
            })
    }

    pub(super) fn player_setup_retry_available(&self) -> bool {
        self.player_setup_issue
            .as_ref()
            .is_some_and(|issue| issue.kind != GuiPlayerSetupIssueKind::NotConfigured)
            && self.pending_operation.is_none()
    }

    pub(super) fn chat_send_unavailable_reason_from_settings(
        &self,
        settings: &StoredClientSettingsMvp,
        session_runtime_available: bool,
    ) -> Option<String> {
        if !legacy_chat_input_enabled(settings) {
            return Some("Chat input is disabled in Chat settings.".to_owned());
        }
        if self.pending_operation.is_some() {
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

    pub(super) fn chat_send_unavailable_reason(&self) -> String {
        self.commands
            .chat_unavailable_reason
            .clone()
            .unwrap_or_else(|| "Chat input is unavailable.".to_owned())
    }

    pub(super) fn chat_send_unavailable_message(&self) -> String {
        let reason = self.chat_send_unavailable_reason();
        if reason.ends_with('.') {
            format!("{reason} The message was not sent.")
        } else {
            format!("{reason}; the message was not sent.")
        }
    }

    pub(super) fn stream_helper_issue_title(&self) -> Option<&'static str> {
        match self.stream_helper.health {
            super::GuiStreamHelperHealth::Healthy => None,
            super::GuiStreamHelperHealth::MissingDownloader => Some("yt-dlp required"),
            super::GuiStreamHelperHealth::MissingJsRuntime => Some("Deno runtime required"),
            super::GuiStreamHelperHealth::Stale => Some("Stream helper update recommended"),
            super::GuiStreamHelperHealth::Broken => Some("Stream helper is broken"),
            super::GuiStreamHelperHealth::UnsupportedPlatform => {
                Some("Manual stream helper setup required")
            }
            super::GuiStreamHelperHealth::ExternalPlayerUnmanaged => {
                Some("External mpv cannot be repaired in place")
            }
        }
    }

    pub(super) fn stream_helper_issue_summary(&self) -> Option<&'static str> {
        match self.stream_helper.health {
            super::GuiStreamHelperHealth::Healthy => None,
            super::GuiStreamHelperHealth::MissingDownloader => {
                Some("Extractor-backed page URLs need yt-dlp before mpv can load them.")
            }
            super::GuiStreamHelperHealth::MissingJsRuntime => {
                Some("Current yt-dlp YouTube extraction also needs a JavaScript runtime.")
            }
            super::GuiStreamHelperHealth::Stale => {
                Some("The managed stream helper should be refreshed before retrying this URL.")
            }
            super::GuiStreamHelperHealth::Broken => {
                Some("The stream helper exists but could not be used by Sorotte.")
            }
            super::GuiStreamHelperHealth::UnsupportedPlatform => Some(
                "Automatic helper installation is not available on this platform yet, but existing helper binaries can still be imported.",
            ),
            super::GuiStreamHelperHealth::ExternalPlayerUnmanaged => Some(
                "This mpv process was started outside Sorotte, so imported helper changes will not reach it until it is relaunched.",
            ),
        }
    }

    pub(super) fn stream_helper_status_title(&self) -> &'static str {
        self.stream_helper_issue_title()
            .unwrap_or("Stream helper status")
    }

    pub(super) fn stream_helper_status_summary(&self) -> String {
        if let Some(summary) = self.stream_helper_issue_summary() {
            return summary.to_owned();
        }
        let downloader_missing = self
            .stream_helper
            .downloader_status
            .as_deref()
            .is_some_and(|status| status.starts_with("Missing "));
        let js_runtime_missing = self
            .stream_helper
            .js_runtime_status
            .as_deref()
            .is_some_and(|status| status.starts_with("Missing "));
        match (downloader_missing, js_runtime_missing) {
            (true, true) => "yt-dlp and Deno are not installed for Sorotte yet.".to_owned(),
            (true, false) => "yt-dlp is not installed for Sorotte yet.".to_owned(),
            (false, true) => "Deno is not installed for Sorotte yet.".to_owned(),
            (false, false) => {
                "yt-dlp and Deno are ready for extractor-backed page URLs.".to_owned()
            }
        }
    }

    pub(super) fn stream_helper_status_available(&self) -> bool {
        self.stream_helper.health != super::GuiStreamHelperHealth::Healthy
            || self.stream_helper.integration_supported
            || self.stream_helper.install_location.is_some()
            || self.stream_helper.downloader_status.is_some()
            || self.stream_helper.js_runtime_status.is_some()
    }

    pub(super) fn media_match_effective_status_label(&self) -> &'static str {
        if !self.media_match.settings.fingerprinting_enabled {
            return "disabled";
        }
        if self.media_matching_background_active() {
            return "indexing";
        }
        self.media_match.health.label()
    }

    pub(super) fn media_match_status_title(&self) -> &'static str {
        if !self.media_match.settings.fingerprinting_enabled {
            return "Media matching disabled";
        }
        if self.media_matching_background_active() {
            return "Media matching indexing";
        }
        match self.media_match.health {
            super::GuiMediaMatchToolHealth::Healthy => "Media matching ready",
            super::GuiMediaMatchToolHealth::MissingFfmpeg => "ffmpeg required",
            super::GuiMediaMatchToolHealth::MissingFfprobe => "ffprobe required",
            super::GuiMediaMatchToolHealth::Broken => "Media matching tools are broken",
        }
    }

    pub(super) fn media_match_status_summary(&self) -> String {
        if !self.media_match.settings.fingerprinting_enabled {
            return if self.media_match.health == super::GuiMediaMatchToolHealth::Healthy {
                "Media Matching is off. Existing cache data is kept; enable it to index local files and match room media.".to_owned()
            } else {
                "Media Matching is off. Import or install ffmpeg and ffprobe before enabling matching.".to_owned()
            };
        }
        if let Some(message) = self.media_match.message.as_ref() {
            return message.clone();
        }
        if self.media_matching_background_active() {
            return "Building the fixed sampled-fast library index for background matching."
                .to_owned();
        }
        if self.media_match.health == super::GuiMediaMatchToolHealth::Healthy {
            return "Fixed sampled-fast audio matching is ready. Exact playlist matches skip library search.".to_owned();
        }
        "Import or install ffmpeg and ffprobe to enable local media matching.".to_owned()
    }

    pub(super) fn media_match_autoplay_policy_summary(&self) -> String {
        match self.media_match.settings.autoplay_policy {
            sorotte_media_match::MediaMatchAutoplayPolicy::DiagnosticsOnly => {
                "Matches are reported but never used for media-match autoplay.".to_owned()
            }
            sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia => {
                "Only exact matches and verified SameCutStrong matches may autoplay; sampled-only probable matches never autoplay.".to_owned()
            }
        }
    }

    pub(super) fn media_matching_background_active(&self) -> bool {
        self.media_match
            .background_status
            .as_deref()
            .is_some_and(|status| {
                let lower = status.to_ascii_lowercase();
                !lower.starts_with("idle")
                    && !lower.starts_with("failed")
                    && !lower.starts_with("canceled")
            })
    }

    pub(super) fn apply_persisted_ui_state(&mut self, persisted_ui_state: &GuiPersistedUiState) {
        persisted_ui_state.apply_to_shell_state(self);
        self.refresh_validation();
        self.refresh_command_availability();
    }

    pub(super) fn remember_media_dialog_directory(&mut self, path: &str) {
        let directory = Path::new(path)
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
            .map(|directory| directory.to_string_lossy().into_owned())
            .or_else(|| normalized_editable_text(path));
        self.last_media_dialog_directory = directory;
    }

    pub(super) fn reset_to_first_run_state(&mut self, settings: StoredClientSettingsMvp) {
        *self = Self::from_stored_settings(&settings);
    }

    pub(super) fn default_selection_from_surfaces(&mut self) {
        self.selection.selected_main_window_user =
            (!self.main_window.users.is_empty()).then_some(0);
        self.set_main_window_playlist_selection(
            self.main_window
                .playlist
                .iter()
                .position(|row| row.is_selected)
                .or_else(|| (!self.main_window.playlist.is_empty()).then_some(0)),
            false,
        );
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

    pub(super) fn select_configuration_tab(&mut self, tab: GuiConfigurationTab) {
        self.selected_configuration_tab = tab;
    }

    pub(super) fn configuration_tab_for_section(section: &str) -> Option<GuiConfigurationTab> {
        match section {
            "Connection" => Some(GuiConfigurationTab::Connection),
            "Readiness" | "Desync" | "Media Search" => Some(GuiConfigurationTab::PlaybackSearch),
            "Privacy" | "Chat" => Some(GuiConfigurationTab::PrivacyChat),
            "OSD" | "System" => Some(GuiConfigurationTab::InterfaceSystem),
            _ => None,
        }
    }

    pub(super) fn normalize_selection(&mut self) {
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
            self.set_main_window_playlist_selection(
                (!self.main_window.playlist.is_empty()).then_some(0),
                false,
            );
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

    pub(super) fn set_main_window_playlist_selection(
        &mut self,
        selected_index: Option<usize>,
        is_local: bool,
    ) {
        self.selection.selected_main_window_playlist = selected_index;
        self.main_window_playlist_selection_is_local = is_local && selected_index.is_some();
    }

    pub(super) fn normalize_selected_menu_action_after_runtime_update(&mut self) {
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

    pub(super) fn set_menu_action_enabled(
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

    pub(super) fn set_menu_action_selected(
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

    pub(super) fn set_runtime_menu_action_override(
        &mut self,
        action_override: MenuActionRuntimeOverride,
    ) {
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

    pub(super) fn clear_runtime_menu_action_override(
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

    pub(super) fn remember_runtime_menu_action_override(
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

    pub(super) fn normalize_runtime_menu_action_overrides_for_settings(
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

    pub(super) fn command_availability_without_runtime_override(
        &self,
    ) -> GuiCommandAvailabilityState {
        let settings = self.configuration.to_stored_settings();
        let busy = self.pending_operation.is_some();
        let chat_unavailable_reason =
            self.chat_send_unavailable_reason_from_settings(&settings, true);
        GuiCommandAvailabilityState {
            can_save_configuration: !busy && self.validation.issues.is_empty(),
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

    pub(super) fn normalize_runtime_command_availability_override_for_current_state(&mut self) {
        let baseline = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override
            .normalize_for_baseline(&baseline);
    }

    pub(super) fn sync_playback_menu_actions_from_runtime_state(&mut self, can_toggle_pause: bool) {
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
            "Shared Playlist",
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

    pub(super) fn sync_dialog_menu_actions_from_runtime_state(&mut self) {
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

    pub(super) fn open_newly_expected_modal_if_needed(
        &mut self,
        previous_tls_prompt_expected: bool,
        _previous_update_notice_expected: bool,
    ) {
        if self.open_modal.is_some() {
            return;
        }
        if self.menus.tls_prompt_expected && !previous_tls_prompt_expected {
            self.open_modal = Some(GuiShellModal::TlsCertificatePrompt);
        }
    }

    pub(super) fn apply_selection_to_surfaces(&mut self) {
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
}
