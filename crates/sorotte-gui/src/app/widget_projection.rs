use sorotte_client_app::app_boundary::state::StoredClientSettings;

use super::shell_state::{
    MainWindowRuntimeSnapshot, MainWindowShellState, MediaSearchWorkflowRuntimeFlags,
    MediaSearchWorkflowShellState, MenuDialogShellState, PublicServerBrowserRuntimeFlags,
    PublicServerBrowserShellState, SorotteGuiShellAppState,
};

impl SorotteGuiShellAppState {
    fn reapply_runtime_main_window_surface_from_snapshot(
        &mut self,
        previous_settings: &StoredClientSettings,
        current_snapshot: &MainWindowRuntimeSnapshot,
    ) {
        let sources = super::playlist_model::GuiPlaylistSources {
            default_source: self
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match,
            plex: &self.plex,
            plugin_enablement: self.plugin_enablement,
        };
        super::main_window_projection::GuiMainWindowProjection {
            main_window: &mut self.main_window,
            selection: &mut self.selection,
            main_window_playlist_selection_is_local: &mut self
                .main_window_playlist_selection_is_local,
            playlist_undo_snapshot: &mut self.playlist_undo_snapshot,
            playlist_source_undo_snapshot: &mut self.playlist_source_undo_snapshot,
            playlist_entry_id_undo_snapshot: &mut self.playlist_entry_id_undo_snapshot,
            pending_local_ready_target: &mut self.pending_local_ready_target,
            menus: &mut self.menus,
            sources,
        }
        .reapply_runtime_main_window_surface_from_snapshot(
            self.commands.can_disconnect_session,
            previous_settings,
            current_snapshot,
        );
    }

    fn preserves_runtime_dialog_expectations(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> (bool, bool) {
        super::configuration_model::preserves_runtime_dialog_expectations(
            &self.menus,
            previous_settings,
        )
    }

    fn preserves_runtime_public_server_surface(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> bool {
        super::configuration_model::preserves_runtime_public_server_surface(
            &self.public_servers,
            previous_settings,
        )
    }

    fn preserves_runtime_media_search_surface(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> bool {
        super::configuration_model::preserves_runtime_media_search_surface(
            &self.media_search,
            previous_settings,
        )
    }

    pub(super) fn sync_derived_surfaces_from_configuration_settings(
        &mut self,
        previous_settings: &StoredClientSettings,
    ) {
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.main_window);
        let preserved_media_index_status = self.media_index_status.clone();
        let settings = self.configuration.to_stored_settings();
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.public_servers.servers.clone());
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_search.directories.clone());
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(previous_settings);
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(previous_settings);
        let preserved_public_server_flags = preserve_public_servers
            .then(|| PublicServerBrowserRuntimeFlags::from_shell_state(&self.public_servers));
        let preserve_media_search = self.preserves_runtime_media_search_surface(previous_settings);
        let preserved_media_search_flags = preserve_media_search
            .then(|| MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_search));
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let tls_prompt_expected = self.menus.tls_prompt_expected;
        let update_notice_expected = self.menus.update_notice_expected;
        let about_dialog_available = self.menus.about_dialog_available;
        self.main_window = MainWindowShellState::from_stored_settings(&settings);
        self.reapply_runtime_main_window_surface_from_snapshot(
            previous_settings,
            &preserved_main_window_runtime_snapshot,
        );
        self.menus = MenuDialogShellState::from_stored_settings(&settings);
        if preserve_tls_prompt_expected {
            self.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.menus.update_notice_expected = update_notice_expected;
        }
        self.menus.about_dialog_available = about_dialog_available;
        self.public_servers = PublicServerBrowserShellState::from_stored_settings(&settings);
        if let Some(servers) = preserved_public_server_rows {
            self.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.public_servers.apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        self.media_search = MediaSearchWorkflowShellState::from_stored_settings(&settings);
        if let Some(directories) = preserved_media_search_directories {
            self.media_search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_search.apply_runtime_flags(runtime_flags);
        }
        self.media_index_status = preserved_media_index_status;
        self.normalize_runtime_menu_action_overrides_for_settings(&settings);
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.normalize_focused_configuration_control();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
    }

    pub(super) fn resync_from_settings(&mut self, settings: StoredClientSettings) {
        let previous_settings = self.configuration.to_stored_settings();
        let active_view = self.active_view;
        let active_application_language = self.active_application_language.clone();
        let active_application_force_gui_prompt = self.active_application_force_gui_prompt;
        let selected_configuration_tab = self.selected_configuration_tab;
        let selected_plugin = self.selected_plugin;
        let open_modal = self.open_modal;
        let selection = self.selection.clone();
        let runtime_menu_action_overrides = self.runtime_menu_action_overrides.clone();
        let runtime_command_availability_override =
            self.runtime_command_availability_override.clone();
        let pending_operation = self.pending_operation.clone();
        let clear_gui_data_confirmation_visible = self.clear_gui_data_confirmation_visible;
        let config_storage = self.config_storage.clone();
        let pending_config_storage_target = self.pending_config_storage_target.clone();
        let pending_saved_server_connect_intent = self.pending_saved_server_connect_intent;
        let outgoing_chat_message = self.outgoing_chat_message.clone();
        let new_main_window_user_draft = self.new_main_window_user_draft.clone();
        let focused_configuration_control = self.focused_configuration_control.clone();
        let public_server_edit_session = self.public_server_edit_session.clone();
        let main_window_user_edit_session = self.main_window_user_edit_session.clone();
        let text_edit_session = self.text_edit_session.clone();
        let playlist_text_edit_session = self.playlist_text_edit_session.clone();
        let playlist_url_edit_session = self.playlist_url_edit_session.clone();
        let media_url_edit_session = self.media_url_edit_session.clone();
        let room_history_edit_session = self.room_history_edit_session.clone();
        let update_check = self.update_check.clone();
        let runtime_validation_issues = self.runtime_validation_issues.clone();
        let notifications = self.notifications.clone();
        let pending_apply_requirements = self.pending_apply_requirements.clone();
        let server_password = self.configuration.server_password.clone();
        let raw_server_password = self.configuration.settings.server_password.clone();
        let last_media_dialog_directory = self.last_media_dialog_directory.clone();
        let last_action_error = self.validation.last_action_error.clone();
        let playlist_undo_snapshot = self.playlist_undo_snapshot.clone();
        let playlist_source_undo_snapshot = self.playlist_source_undo_snapshot.clone();
        let playlist_entry_id_undo_snapshot = self.playlist_entry_id_undo_snapshot.clone();
        let playlist_shuffle_nonce = self.playlist_shuffle_nonce;
        let media_index_status = self.media_index_status.clone();
        let player_setup_issue = self.player_setup_issue.clone();
        let plex = self.plex.clone();
        let saved_configuration = self.saved_configuration.clone();
        let tls_prompt_expected = self.menus.tls_prompt_expected;
        let update_notice_expected = self.menus.update_notice_expected;
        let about_dialog_available = self.menus.about_dialog_available;
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.main_window);
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(&previous_settings);
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.public_servers.servers.clone());
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(&previous_settings);
        let preserved_public_server_flags = preserve_public_servers
            .then(|| PublicServerBrowserRuntimeFlags::from_shell_state(&self.public_servers));
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_search.directories.clone());
        let preserve_media_search = self.preserves_runtime_media_search_surface(&previous_settings);
        let preserved_media_search_flags = preserve_media_search
            .then(|| MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_search));

        *self = Self::from_stored_settings(&settings);
        self.active_view = active_view;
        self.active_application_language = active_application_language;
        self.active_application_force_gui_prompt = active_application_force_gui_prompt;
        self.selected_configuration_tab = selected_configuration_tab;
        self.selected_plugin = selected_plugin;
        self.open_modal = open_modal;
        self.selection = selection;
        self.runtime_menu_action_overrides = runtime_menu_action_overrides;
        self.runtime_command_availability_override = runtime_command_availability_override;
        self.pending_operation = pending_operation;
        self.clear_gui_data_confirmation_visible = clear_gui_data_confirmation_visible;
        self.config_storage = config_storage;
        self.pending_config_storage_target = pending_config_storage_target;
        self.pending_saved_server_connect_intent = pending_saved_server_connect_intent;
        self.outgoing_chat_message = outgoing_chat_message;
        self.new_main_window_user_draft = new_main_window_user_draft;
        self.focused_configuration_control = focused_configuration_control;
        self.public_server_edit_session = public_server_edit_session;
        self.main_window_user_edit_session = main_window_user_edit_session;
        self.text_edit_session = text_edit_session;
        self.playlist_text_edit_session = playlist_text_edit_session;
        self.playlist_url_edit_session = playlist_url_edit_session;
        self.media_url_edit_session = media_url_edit_session;
        self.room_history_edit_session = room_history_edit_session;
        self.update_check = update_check;
        self.runtime_validation_issues = runtime_validation_issues;
        self.notifications = notifications;
        self.pending_apply_requirements = pending_apply_requirements;
        self.configuration.settings.server_password = raw_server_password;
        self.configuration.server_password = server_password;
        self.last_media_dialog_directory = last_media_dialog_directory;
        self.playlist_undo_snapshot = playlist_undo_snapshot;
        self.playlist_source_undo_snapshot = playlist_source_undo_snapshot;
        self.playlist_entry_id_undo_snapshot = playlist_entry_id_undo_snapshot;
        self.playlist_shuffle_nonce = playlist_shuffle_nonce;
        self.media_index_status = media_index_status;
        self.player_setup_issue = player_setup_issue;
        self.plex = plex;
        self.saved_configuration = saved_configuration;
        if preserve_tls_prompt_expected {
            self.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.menus.update_notice_expected = update_notice_expected;
        }
        self.menus.about_dialog_available = about_dialog_available;
        if let Some(servers) = preserved_public_server_rows {
            self.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.public_servers.apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        if let Some(directories) = preserved_media_search_directories {
            self.media_search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_search.apply_runtime_flags(runtime_flags);
        }
        self.normalize_runtime_menu_action_overrides_for_settings(&settings);
        self.reapply_runtime_main_window_surface_from_snapshot(
            &previous_settings,
            &preserved_main_window_runtime_snapshot,
        );
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.normalize_focused_configuration_control();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.validation.last_action_error = last_action_error;
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.commands.can_toggle_pause);
    }

    pub(super) fn has_unsaved_configuration_changes(&self) -> bool {
        self.pending_config_storage_target.is_some()
            || self
                .configuration
                .has_unsaved_changes_against(&self.saved_configuration)
    }
}
