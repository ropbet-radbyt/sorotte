use super::GuiRuntimeState;
use crate::app::shell_state::*;
use sorotte_client_app::app_boundary::state::StoredClientSettings;

impl GuiRuntimeState {
    pub(super) fn reapply_runtime_main_window_surface_from_snapshot(
        &mut self,
        previous_settings: &StoredClientSettings,
        current_snapshot: &MainWindowRuntimeSnapshot,
    ) {
        let sources = crate::app::playlist_model::GuiPlaylistSources {
            default_source: self
                .playlist
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match.model,
            plex: &self.plex.model,
            plugin_enablement: self.settings.plugin_enablement,
        };
        crate::app::main_window_projection::GuiMainWindowProjection {
            main_window: &mut self.playlist.main_window,
            selection: &mut self.playlist.selection,
            main_window_playlist_selection_is_local: &mut self.playlist.selection_is_local,
            playlist_undo_snapshot: &mut self.playlist.undo_snapshot,
            playlist_source_undo_snapshot: &mut self.playlist.source_undo_snapshot,
            playlist_entry_id_undo_snapshot: &mut self.playlist.entry_id_undo_snapshot,
            pending_local_ready_target: &mut self.session.pending_local_ready_target,
            menus: &mut self.session.menus,
            sources,
        }
        .reapply_runtime_main_window_surface_from_snapshot(
            self.session.commands.can_disconnect_session,
            previous_settings,
            current_snapshot,
        );
    }
    fn preserves_runtime_dialog_expectations(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> (bool, bool) {
        crate::app::configuration_model::preserves_runtime_dialog_expectations(
            &self.session.menus,
            previous_settings,
        )
    }
    fn preserves_runtime_public_server_surface(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> bool {
        crate::app::configuration_model::preserves_runtime_public_server_surface(
            &self.session.public_servers,
            previous_settings,
        )
    }
    fn preserves_runtime_media_search_surface(
        &self,
        previous_settings: &StoredClientSettings,
    ) -> bool {
        crate::app::configuration_model::preserves_runtime_media_search_surface(
            &self.media_resolution.search,
            previous_settings,
        )
    }
    pub(super) fn resync_from_settings(&mut self, settings: StoredClientSettings) {
        let previous_settings = self.settings.draft.to_stored_settings();

        let active_application_language = self.settings.active_application_language.clone();
        let active_application_force_gui_prompt = self.settings.active_application_force_gui_prompt;

        let selection = self.playlist.selection.clone();
        let runtime_menu_action_overrides = self.session.menu_overrides.clone();
        let runtime_command_availability_override = self.session.command_overrides.clone();
        let pending_operation = self.session.pending_operation.clone();

        let config_storage = self.settings.config_storage.clone();
        let pending_config_storage_target = self.settings.pending_storage_target.clone();
        let pending_saved_server_connect_intent = self.session.pending_saved_server_connect_intent;
        let outgoing_chat_message = self.session.outgoing_chat_message.clone();

        let update_check = self.updates.model.clone();
        let runtime_validation_issues = self.settings.runtime_validation_issues.clone();

        let server_password = self.settings.draft.server_password.clone();
        let raw_server_password = self.settings.draft.settings.server_password.clone();
        let last_media_dialog_directory = self.media_resolution.last_dialog_directory.clone();
        let last_action_error = self.settings.validation.last_action_error.clone();
        let playlist_undo_snapshot = self.playlist.undo_snapshot.clone();
        let playlist_source_undo_snapshot = self.playlist.source_undo_snapshot.clone();
        let playlist_entry_id_undo_snapshot = self.playlist.entry_id_undo_snapshot.clone();
        let playlist_shuffle_nonce = self.playlist.shuffle_nonce;
        let media_index_status = self.media_resolution.index_status.clone();
        let player_setup_issue = self.player.setup_issue.clone();
        let plex = self.plex.model.clone();
        let saved_configuration = self.settings.saved.clone();
        let tls_prompt_expected = self.session.menus.tls_prompt_expected;
        let update_notice_expected = self.session.menus.update_notice_expected;
        let about_dialog_available = self.session.menus.about_dialog_available;
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.playlist.main_window);
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(&previous_settings);
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.session.public_servers.servers.clone());
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(&previous_settings);
        let preserved_public_server_flags = preserve_public_servers.then(|| {
            PublicServerBrowserRuntimeFlags::from_shell_state(&self.session.public_servers)
        });
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_resolution.search.directories.clone());
        let preserve_media_search = self.preserves_runtime_media_search_surface(&previous_settings);
        let preserved_media_search_flags = preserve_media_search.then(|| {
            MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_resolution.search)
        });

        *self = Self::from_stored_settings(&settings);

        self.settings.active_application_language = active_application_language;
        self.settings.active_application_force_gui_prompt = active_application_force_gui_prompt;

        self.playlist.selection = selection;
        self.session.menu_overrides = runtime_menu_action_overrides;
        self.session.command_overrides = runtime_command_availability_override;
        self.session.pending_operation = pending_operation;

        self.settings.config_storage = config_storage;
        self.settings.pending_storage_target = pending_config_storage_target;
        self.session.pending_saved_server_connect_intent = pending_saved_server_connect_intent;
        self.session.outgoing_chat_message = outgoing_chat_message;

        self.updates.model = update_check;
        self.settings.runtime_validation_issues = runtime_validation_issues;

        self.settings.draft.settings.server_password = raw_server_password;
        self.settings.draft.server_password = server_password;
        self.media_resolution.last_dialog_directory = last_media_dialog_directory;
        self.playlist.undo_snapshot = playlist_undo_snapshot;
        self.playlist.source_undo_snapshot = playlist_source_undo_snapshot;
        self.playlist.entry_id_undo_snapshot = playlist_entry_id_undo_snapshot;
        self.playlist.shuffle_nonce = playlist_shuffle_nonce;
        self.media_resolution.index_status = media_index_status;
        self.player.setup_issue = player_setup_issue;
        self.plex.model = plex;
        self.settings.saved = saved_configuration;
        if preserve_tls_prompt_expected {
            self.session.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.session.menus.update_notice_expected = update_notice_expected;
        }
        self.session.menus.about_dialog_available = about_dialog_available;
        if let Some(servers) = preserved_public_server_rows {
            self.session.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.session
                .public_servers
                .apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        if let Some(directories) = preserved_media_search_directories {
            self.media_resolution.search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_resolution
                .search
                .apply_runtime_flags(runtime_flags);
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

        self.settings.validation.last_action_error = last_action_error;
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.session.commands.can_toggle_pause);
    }

    pub(super) fn normalize_runtime_menu_action_overrides_for_settings(
        &mut self,
        settings: &StoredClientSettings,
    ) {
        crate::app::configuration_model::normalize_runtime_menu_action_overrides_for_settings(
            &mut self.session.menu_overrides,
            settings,
        );
    }

    fn selected_public_server_address(&self) -> Option<&str> {
        self.session
            .public_servers
            .servers
            .iter()
            .find(|row| row.is_selected)
            .map(|row| row.address.as_str())
    }
    pub(super) fn restore_selected_public_server_address(&mut self, address: Option<&str>) {
        crate::app::configuration_model::restore_selected_public_server_address(
            &mut self.session.public_servers,
            address,
        );
    }
    pub(super) fn sync_dialog_menu_actions_from_runtime_state(&mut self) {
        for action_override in &self.session.menu_overrides {
            if let Some(action) = self.session.menus.action_mut(action_override.id) {
                action.enabled = action_override.enabled;
            }
        }
        let about = self.session.menus.about_dialog_available;
        if let Some(action) = self.session.menus.action_mut(MenuActionId::About) {
            action.enabled = about;
        }
    }
    pub(super) fn normalize_selected_menu_action_after_runtime_update(&mut self) {
        crate::app::selection_projection::normalize_selected_menu_action_after_runtime_update(
            &self.session.menus,
            &mut self.playlist.selection,
        );
    }
    fn normalize_runtime_command_availability_override_for_current_state(&mut self) {
        let baseline = self.command_availability_without_runtime_override();
        self.session
            .command_overrides
            .normalize_for_baseline(&baseline);
    }
    pub(super) fn settle_persisted_configuration(&mut self, settings: StoredClientSettings) {
        self.resync_from_settings(settings.clone());
        self.settings.saved = settings;
        self.settings.draft.settings.server_password = self.settings.saved.server_password.clone();
        self.settings.draft.server_password = SecretDraft::Unchanged;
    }
}

impl GuiRuntimeState {
    pub(super) fn sync_derived_surfaces_from_configuration_settings(
        &mut self,
        previous_settings: &StoredClientSettings,
    ) {
        let preserved_main_window_runtime_snapshot =
            MainWindowRuntimeSnapshot::from_shell_state(&self.playlist.main_window);
        let preserved_media_index_status = self.media_resolution.index_status.clone();
        let settings = self.settings.draft.to_stored_settings();
        let preserved_public_server_rows = (previous_settings.public_servers
            == settings.public_servers)
            .then(|| self.session.public_servers.servers.clone());
        let preserved_media_search_directories = (previous_settings.media_search_directories
            == settings.media_search_directories)
            .then(|| self.media_resolution.search.directories.clone());
        let (preserve_tls_prompt_expected, preserve_update_notice_expected) =
            self.preserves_runtime_dialog_expectations(previous_settings);
        let preserve_public_servers =
            self.preserves_runtime_public_server_surface(previous_settings);
        let preserved_public_server_flags = preserve_public_servers.then(|| {
            PublicServerBrowserRuntimeFlags::from_shell_state(&self.session.public_servers)
        });
        let preserve_media_search = self.preserves_runtime_media_search_surface(previous_settings);
        let preserved_media_search_flags = preserve_media_search.then(|| {
            MediaSearchWorkflowRuntimeFlags::from_shell_state(&self.media_resolution.search)
        });
        let selected_public_server_address =
            self.selected_public_server_address().map(str::to_owned);
        let tls_prompt_expected = self.session.menus.tls_prompt_expected;
        let update_notice_expected = self.session.menus.update_notice_expected;
        let about_dialog_available = self.session.menus.about_dialog_available;
        self.playlist.main_window = MainWindowShellState::from_stored_settings(&settings);
        self.reapply_runtime_main_window_surface_from_snapshot(
            previous_settings,
            &preserved_main_window_runtime_snapshot,
        );
        self.session.menus = MenuDialogShellState::from_stored_settings(&settings);
        if preserve_tls_prompt_expected {
            self.session.menus.tls_prompt_expected = tls_prompt_expected;
        }
        if preserve_update_notice_expected {
            self.session.menus.update_notice_expected = update_notice_expected;
        }
        self.session.menus.about_dialog_available = about_dialog_available;
        self.session.public_servers =
            PublicServerBrowserShellState::from_stored_settings(&settings);
        if let Some(servers) = preserved_public_server_rows {
            self.session.public_servers.servers = servers;
        }
        if let Some(runtime_flags) = preserved_public_server_flags {
            self.session
                .public_servers
                .apply_runtime_flags(runtime_flags);
        }
        self.restore_selected_public_server_address(selected_public_server_address.as_deref());
        self.media_resolution.search =
            MediaSearchWorkflowShellState::from_stored_settings(&settings);
        if let Some(directories) = preserved_media_search_directories {
            self.media_resolution.search.directories = directories;
        }
        if let Some(runtime_flags) = preserved_media_search_flags {
            self.media_resolution
                .search
                .apply_runtime_flags(runtime_flags);
        }
        self.media_resolution.index_status = preserved_media_index_status;
        self.normalize_runtime_menu_action_overrides_for_settings(&settings);
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.refresh_validation();
        self.normalize_runtime_command_availability_override_for_current_state();
        self.refresh_command_availability();
        self.sync_playback_menu_actions_from_runtime_state(self.session.commands.can_toggle_pause);
    }
}
