use std::path::Path;

use sorotte_client_app::app_boundary::state::StoredClientSettings;

use super::shell_state::{
    GuiCommandAvailabilityState, GuiConfigurationTab, GuiPlayerSetupIssueKind, GuiPluginSelection,
    GuiSavedSessionConnectTarget, GuiShellAction, GuiShellModal, GuiShellView, MenuActionId,
    SettingId, SorotteGuiShellAppState,
};
use super::support::normalized_editable_text;
use super::ui_state::GuiPersistedUiState;

impl SorotteGuiShellAppState {
    pub(super) fn from_stored_settings(settings: &StoredClientSettings) -> Self {
        let runtime = crate::app::runtime_state::GuiRuntimeState::from_stored_settings(settings);
        Self {
            active_view: GuiShellView::Setup,
            active_application_language: runtime.settings.active_application_language,
            active_application_force_gui_prompt: runtime
                .settings
                .active_application_force_gui_prompt,
            selected_configuration_tab: GuiConfigurationTab::Connection,
            selected_plugin: GuiPluginSelection::default(),
            plugin_enablement: runtime.settings.plugin_enablement,
            open_modal: None,
            selection: runtime.playlist.selection,
            main_window_playlist_selection_is_local: runtime.playlist.selection_is_local,
            runtime_menu_action_overrides: runtime.session.menu_overrides,
            runtime_command_availability_override: runtime.session.command_overrides,
            config_storage: runtime.settings.config_storage,
            commands: runtime.session.commands,
            pending_operation: runtime.session.pending_operation,
            clear_gui_data_confirmation_visible: false,
            pending_config_storage_target: runtime.settings.pending_storage_target,
            pending_local_ready_target: runtime.session.pending_local_ready_target,
            pending_saved_server_connect_intent: runtime
                .session
                .pending_saved_server_connect_intent,
            outgoing_chat_message: runtime.session.outgoing_chat_message,
            main_window_room_change_expanded: false,
            new_main_window_user_draft: String::new(),
            focused_configuration_control: None,
            public_server_edit_session: None,
            main_window_user_edit_session: None,
            text_edit_session: None,
            playlist_text_edit_session: None,
            playlist_url_edit_session: None,
            plex_playlist_search: runtime.plex.playlist_search,
            media_url_edit_session: None,
            controlled_room_create_session: None,
            controller_auth_edit_session: None,
            room_history_edit_session: None,
            update_check: runtime.updates.model,
            runtime_validation_issues: runtime.settings.runtime_validation_issues,
            notifications: Vec::new(),
            pending_apply_requirements: Vec::new(),
            validation: runtime.settings.validation,
            last_media_dialog_directory: runtime.media_resolution.last_dialog_directory,
            playlist_undo_snapshot: runtime.playlist.undo_snapshot,
            playlist_source_undo_snapshot: runtime.playlist.source_undo_snapshot,
            playlist_entry_id_undo_snapshot: runtime.playlist.entry_id_undo_snapshot,
            playlist_shuffle_nonce: runtime.playlist.shuffle_nonce,
            media_index_status: runtime.media_resolution.index_status,
            player_setup_issue: runtime.player.setup_issue,
            seek_preparation: runtime.player.seek_preparation,
            seek_preparation_degraded_reason: runtime.player.seek_preparation_degraded_reason,
            stream_helper: runtime.player.stream_helper,
            stream_helper_remediation: runtime.player.stream_helper_remediation,
            media_match: runtime.media_match.model,
            media_match_remediation: runtime.media_match.remediation,
            plex: runtime.plex.model,
            saved_configuration: runtime.settings.saved,
            configuration: runtime.settings.draft,
            main_window: runtime.playlist.main_window,
            menus: runtime.session.menus,
            public_servers: runtime.session.public_servers,
            media_search: runtime.media_resolution.search,
        }
    }

    pub(super) fn saved_session_connect_target(&self) -> Option<GuiSavedSessionConnectTarget> {
        super::configuration_model::saved_session_connect_target(&self.configuration)
    }
    pub(super) fn saved_session_connect_button_label(&self) -> &'static str {
        if self.commands.can_disconnect_session {
            "Reconnect"
        } else {
            "Connect"
        }
    }

    pub(super) fn connect_blocked_by_player_setup_issue(&self) -> bool {
        super::configuration_model::connect_blocked_by_player_setup_issue(
            &self.configuration,
            &self.player_setup_issue,
        )
    }
    pub(super) fn player_setup_connect_block_message(&self) -> Option<String> {
        if !self.connect_blocked_by_player_setup_issue() {
            return None;
        }
        if self
            .player_setup_issue
            .as_ref()
            .is_some_and(|issue| super::mpv_launch::message_requires_mpv_upgrade(&issue.message))
        {
            return Some(format!(
                "Upgrade mpv to {} or newer, then retry mpv before connecting.",
                sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
            ));
        }
        Some(
            "Set up mpv before connecting. Use Auto-detect, Choose mpv.exe, or Retry mpv after updating Player Path."
                .to_owned(),
        )
    }

    pub(super) fn player_setup_issue_title(&self) -> Option<&'static str> {
        self.player_setup_issue.as_ref().map(|issue| {
            if super::mpv_launch::message_requires_mpv_upgrade(&issue.message) {
                return "mpv upgrade required";
            }
            match issue.kind {
                GuiPlayerSetupIssueKind::NotConfigured => "mpv setup required",
                GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer => "Unsupported player",
                GuiPlayerSetupIssueKind::MissingBinary => "Configured mpv not found",
                GuiPlayerSetupIssueKind::LaunchFailed => "mpv failed to launch",
                GuiPlayerSetupIssueKind::IpcAttachFailed => "mpv did not respond",
                GuiPlayerSetupIssueKind::ExitedAfterLaunch => "mpv closed unexpectedly",
                GuiPlayerSetupIssueKind::PlayerSettingsDegraded => {
                    "mpv streaming settings incomplete"
                }
                GuiPlayerSetupIssueKind::BridgeDegraded => "mpv Chat/OSD integration unavailable",
            }
        })
    }

    pub(super) fn player_setup_issue_summary(&self) -> Option<&'static str> {
        self.player_setup_issue.as_ref().map(|issue| {
            if super::mpv_launch::message_requires_mpv_upgrade(&issue.message) {
                return "The configured mpv does not meet Sorotte's supported-version requirement and must be upgraded.";
            }
            match issue.kind {
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
                GuiPlayerSetupIssueKind::PlayerSettingsDegraded => {
                    "mpv is ready, but some streaming settings could not be applied to the active media."
                }
                GuiPlayerSetupIssueKind::BridgeDegraded => {
                    "mpv is ready, but Chat/OSD integration could not be configured."
                }
            }
        })
    }

    pub(super) fn player_setup_retry_label(&self) -> &'static str {
        match self.player_setup_issue.as_ref().map(|issue| issue.kind) {
            Some(GuiPlayerSetupIssueKind::BridgeDegraded) => "Retry Chat/OSD integration",
            Some(GuiPlayerSetupIssueKind::PlayerSettingsDegraded) => "Retry mpv settings",
            _ => "Retry mpv",
        }
    }

    pub(super) fn player_setup_retry_action(&self) -> GuiShellAction {
        match self.player_setup_issue.as_ref().map(|issue| issue.kind) {
            Some(GuiPlayerSetupIssueKind::PlayerSettingsDegraded) => {
                GuiShellAction::RetryPlayerSettings
            }
            Some(GuiPlayerSetupIssueKind::BridgeDegraded) => {
                GuiShellAction::RetryChatOsdIntegration
            }
            _ => GuiShellAction::RetryPlayerLaunch,
        }
    }

    pub(super) fn player_setup_retry_available(&self) -> bool {
        self.player_setup_issue
            .as_ref()
            .is_some_and(|issue| issue.retry_available)
            && self.pending_operation.is_none()
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
        if !self
            .plugin_enablement
            .enabled_for(super::GuiPluginSelection::StreamSupport)
        {
            return "Stream Support disabled";
        }
        self.stream_helper_issue_title()
            .unwrap_or("Stream helper status")
    }

    pub(super) fn stream_helper_status_summary(&self) -> String {
        if !self
            .plugin_enablement
            .enabled_for(super::GuiPluginSelection::StreamSupport)
        {
            return "Stream Support is off. Installed helper tools are kept; enable it to handle extractor-backed URLs.".to_owned();
        }
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
        if !self
            .plugin_enablement
            .enabled_for(super::GuiPluginSelection::MediaMatching)
        {
            return "disabled";
        }
        if !self.media_match.settings.fingerprinting_enabled {
            return "disabled";
        }
        if self.media_matching_background_active() {
            return "indexing";
        }
        self.media_match.health.label()
    }

    pub(super) fn media_match_status_title(&self) -> &'static str {
        if !self
            .plugin_enablement
            .enabled_for(super::GuiPluginSelection::MediaMatching)
        {
            return "Media Matching disabled";
        }
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
        if !self
            .plugin_enablement
            .enabled_for(super::GuiPluginSelection::MediaMatching)
        {
            return "Media Matching is off. Existing tools, cache data, and matching settings are kept.".to_owned();
        }
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

    pub(super) fn reset_to_first_run_state(&mut self, settings: StoredClientSettings) {
        *self = Self::from_stored_settings(&settings);
    }

    pub(super) fn select_configuration_tab(&mut self, tab: GuiConfigurationTab) {
        self.selected_configuration_tab = tab;
    }

    pub(super) fn configuration_tab_for_setting(id: SettingId) -> GuiConfigurationTab {
        match id.section_automation_id() {
            "settings.section.connection" => GuiConfigurationTab::Connection,
            "settings.section.playback"
            | "settings.section.sync"
            | "settings.section.streaming"
            | "settings.section.media_library" => GuiConfigurationTab::PlaybackSearch,
            "settings.section.privacy" | "settings.section.chat" => {
                GuiConfigurationTab::PrivacyChat
            }
            "settings.section.osd" | "settings.section.general" => {
                GuiConfigurationTab::InterfaceSystem
            }
            _ => unreachable!("every SettingId maps to a settings section"),
        }
    }

    pub(super) fn normalize_selection(&mut self) {
        super::selection_projection::normalize_selection(
            &self.main_window,
            &self.media_search,
            &self.menus,
            &mut self.selection,
            &mut self.main_window_playlist_selection_is_local,
        );
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
        super::selection_projection::normalize_selected_menu_action_after_runtime_update(
            &self.menus,
            &mut self.selection,
        );
    }
    pub(super) fn set_menu_action_enabled(&mut self, action_id: MenuActionId, enabled: bool) {
        let Some(action) = self.menus.action_mut(action_id) else {
            return;
        };
        action.enabled = enabled;
    }

    pub(super) fn menu_action_available_now(&self, action_id: MenuActionId) -> bool {
        let playback_controls_available =
            self.pending_operation.is_none() && !self.main_window.playlist.is_empty();
        match action_id {
            MenuActionId::OpenMedia => {
                self.pending_operation.is_none() && self.media_open_runtime_available()
            }
            MenuActionId::Play | MenuActionId::Pause | MenuActionId::TogglePause => {
                playback_controls_available && self.main_window.playback.can_toggle_pause
            }
            MenuActionId::Seek => playback_controls_available && self.main_window.playback.can_seek,
            MenuActionId::UndoSeek => {
                playback_controls_available && self.main_window.playback.can_undo_seek
            }
            MenuActionId::SharedPlaylist => {
                self.pending_operation.is_none() && self.main_window.playback.can_manage_playlist
            }
            MenuActionId::SetOffset => {
                playback_controls_available && self.main_window.playback.can_set_offset
            }
            _ => self
                .menus
                .action(action_id)
                .is_some_and(|action| action.enabled),
        }
    }

    pub(super) fn set_menu_action_checked(&mut self, action_id: MenuActionId, checked: bool) {
        let Some(action) = self.menus.action_mut(action_id) else {
            return;
        };
        action.is_checked = checked;
    }

    pub(super) fn normalize_runtime_menu_action_overrides_for_settings(
        &mut self,
        settings: &StoredClientSettings,
    ) {
        super::configuration_model::normalize_runtime_menu_action_overrides_for_settings(
            &mut self.runtime_menu_action_overrides,
            settings,
        );
    }

    pub(super) fn command_availability_without_runtime_override(
        &self,
    ) -> GuiCommandAvailabilityState {
        super::configuration_model::GuiCommandAvailabilityContext {
            configuration: &self.configuration,
            saved_configuration: &self.saved_configuration,
            pending_config_storage_target: &self.pending_config_storage_target,
            pending_operation: &self.pending_operation,
            player_setup_issue: &self.player_setup_issue,
            validation: &self.validation,
            main_window: &self.main_window,
            public_servers: &self.public_servers,
            media_search: &self.media_search,
        }
        .command_availability_without_runtime_override()
    }
    pub(super) fn normalize_runtime_command_availability_override_for_current_state(&mut self) {
        let baseline = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override
            .normalize_for_baseline(&baseline);
    }

    pub(super) fn sync_playback_menu_actions_from_runtime_state(&mut self, can_toggle_pause: bool) {
        super::selection_projection::sync_playback_menu_actions_from_runtime_state(
            &self.main_window,
            &mut self.menus,
            &mut self.selection,
            &self.pending_operation,
            can_toggle_pause,
        );
        self.apply_selection_to_surfaces();
    }
    pub(super) fn sync_dialog_menu_actions_from_runtime_state(&mut self) {
        let runtime_menu_action_overrides = self.runtime_menu_action_overrides.clone();
        for action_override in runtime_menu_action_overrides {
            self.set_menu_action_enabled(action_override.id, action_override.enabled);
        }
        self.set_menu_action_enabled(MenuActionId::About, self.menus.about_dialog_available);
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
        super::selection_projection::apply_selection_to_surfaces(
            &self.selection,
            &mut self.main_window,
            &mut self.menus,
            &mut self.media_search,
        );
    }
}

#[cfg(test)]
mod mpv_version_presentation_tests {
    use super::*;
    use crate::app::shell_state::GuiPlayerSetupIssue;
    use sorotte_player_api::PlayerError;

    #[test]
    fn unsupported_mpv_version_has_upgrade_specific_setup_guidance() {
        let version_error = PlayerError::OperationFailed(format!(
            "Sorotte requires mpv {} or newer, but the connected mpv reports mpv 0.40.0; upgrade mpv and try again",
            sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
        ));
        let detail = crate::app::mpv_launch::mpv_upgrade_required_diagnostic(&version_error)
            .expect("unsupported version errors should produce upgrade guidance");
        let mut state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
        state.player_setup_issue = Some(GuiPlayerSetupIssue {
            kind: GuiPlayerSetupIssueKind::IpcAttachFailed,
            message: format!("mpv JSON IPC attach failed: {detail}"),
            retry_available: true,
        });

        assert_eq!(
            state.player_setup_issue_title(),
            Some("mpv upgrade required")
        );
        assert_eq!(
            state.player_setup_issue_summary(),
            Some(
                "The configured mpv does not meet Sorotte's supported-version requirement and must be upgraded."
            )
        );
        let block_message = state
            .player_setup_connect_block_message()
            .expect("an unsupported player should block connection");
        assert!(block_message.contains(&format!(
            "Upgrade mpv to {} or newer",
            sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
        )));
        assert!(block_message.contains("retry mpv"));
    }
}
