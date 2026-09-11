use super::GuiRuntimeState;
use crate::app::configuration_model;
use crate::app::shell_state::*;
use sorotte_client_app::app_boundary::state::StoredClientSettings;

impl GuiRuntimeState {
    pub(in crate::app) fn saved_session_connect_target(
        &self,
    ) -> Option<GuiSavedSessionConnectTarget> {
        configuration_model::saved_session_connect_target(&self.settings.draft)
    }
    pub(in crate::app) fn connect_blocked_by_player_setup_issue(&self) -> bool {
        configuration_model::connect_blocked_by_player_setup_issue(
            &self.settings.draft,
            &self.player.setup_issue,
        )
    }
    pub(in crate::app) fn submitted_saved_server_connect_settings(
        &self,
        intent: GuiSavedServerConnectIntent,
    ) -> StoredClientSettings {
        configuration_model::submitted_saved_server_connect_settings(
            &self.settings.draft,
            &self.settings.saved,
            intent,
        )
    }
    pub(in crate::app) fn chat_send_unavailable_reason_from_settings(
        &self,
        settings: &StoredClientSettings,
        session_runtime_available: bool,
    ) -> Option<String> {
        configuration_model::chat_send_unavailable_reason_from_settings(
            &self.session.pending_operation,
            settings,
            session_runtime_available,
        )
    }
    pub(in crate::app) fn has_unsaved_configuration_changes(&self) -> bool {
        self.settings.pending_storage_target.is_some()
            || self
                .settings
                .draft
                .has_unsaved_changes_against(&self.settings.saved)
    }
    pub(in crate::app) fn runtime_language_tag(&self) -> &'static str {
        crate::app::runtime_localization::normalized_runtime_language_tag_or_default(
            self.settings.active_application_language.as_deref(),
        )
    }
    pub(super) fn refresh_validation(&mut self) {
        let mut issues = configuration_model::GuiConfigurationContext {
            configuration: &self.settings.draft,
            public_servers: &self.session.public_servers,
            media_search: &self.media_resolution.search,
        }
        .validation_issues();
        issues.extend(self.settings.runtime_validation_issues.iter().cloned());
        self.settings.validation.issues = issues;
        self.refresh_command_availability();
    }
    pub(super) fn clear_action_error_and_refresh(&mut self) {
        self.settings.validation.last_action_error = None;
        self.refresh_validation();
    }
    pub(super) fn record_action_error(&mut self, message: impl Into<String>) -> bool {
        self.settings.validation.last_action_error = Some(
            crate::app::runtime_localization::localize_gui_runtime_message(
                &message.into(),
                Some(self.runtime_language_tag()),
            ),
        );
        self.refresh_validation();
        false
    }
}

impl GuiRuntimeState {
    pub(in crate::app) fn command_availability_without_runtime_override(
        &self,
    ) -> GuiCommandAvailabilityState {
        crate::app::configuration_model::GuiCommandAvailabilityContext {
            configuration: &self.settings.draft,
            saved_configuration: &self.settings.saved,
            pending_config_storage_target: &self.settings.pending_storage_target,
            pending_operation: &self.session.pending_operation,
            player_setup_issue: &self.player.setup_issue,
            validation: &self.settings.validation,
            main_window: &self.playlist.main_window,
            public_servers: &self.session.public_servers,
            media_search: &self.media_resolution.search,
        }
        .command_availability_without_runtime_override()
    }
    pub(super) fn refresh_command_availability(&mut self) {
        self.session.commands = self.command_availability_without_runtime_override();
        self.session
            .command_overrides
            .apply_to(&mut self.session.commands);
        self.sync_playback_menu_actions_from_runtime_state(self.session.commands.can_toggle_pause);
    }
}
impl GuiRuntimeState {
    pub(super) fn apply_selection_to_surfaces(&mut self) {
        crate::app::selection_projection::apply_selection_to_surfaces(
            &self.playlist.selection,
            &mut self.playlist.main_window,
            &mut self.session.menus,
            &mut self.media_resolution.search,
        );
    }
    pub(super) fn sync_playback_menu_actions_from_runtime_state(&mut self, can_toggle_pause: bool) {
        crate::app::selection_projection::sync_playback_menu_actions_from_runtime_state(
            &self.playlist.main_window,
            &mut self.session.menus,
            &mut self.playlist.selection,
            &self.session.pending_operation,
            can_toggle_pause,
        );
        self.apply_selection_to_surfaces();
    }
}
