use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_service_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::SelectMenuAction {
                section_index,
                action_index,
            } => {
                let Some(section) = self.menus.sections.get(section_index) else {
                    return self
                        .record_action_error("No menu section exists at the requested index.");
                };
                if action_index >= section.actions.len() {
                    return self
                        .record_action_error("No menu action exists at the requested index.");
                }
                self.selection.selected_menu_action = Some((section_index, action_index));
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SelectMediaSearchDirectory(index) => {
                if index >= self.media_search.directories.len() {
                    return self.record_action_error(
                        "No media-search directory exists at the requested index.",
                    );
                }
                self.selection.selected_media_search_directory = Some(index);
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SelectPlugin(plugin) => {
                self.selected_plugin = plugin;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::MoveSelectedMediaSearchDirectoryUp => {
                self.move_selected_media_search_directory(-1)
            }
            GuiShellAction::MoveSelectedMediaSearchDirectoryDown => {
                self.move_selected_media_search_directory(1)
            }
            GuiShellAction::RemoveSelectedMediaSearchDirectory => {
                self.remove_selected_media_search_directory()
            }
            GuiShellAction::EditConfigurationText {
                section,
                label,
                value,
            } => {
                let previous_settings = self.configuration.to_stored_settings();
                let applied = self.configuration.apply_text_value(section, label, &value);
                if applied {
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                } else {
                    return self
                        .record_action_error("Configuration text control could not be updated.");
                }
                applied
            }
            GuiShellAction::EditConfigurationBool {
                section,
                label,
                value,
            } => {
                let previous_settings = self.configuration.to_stored_settings();
                let applied = self.configuration.apply_bool_value(section, label, value);
                if applied {
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                } else {
                    return self.record_action_error(
                        "Configuration checkbox control could not be updated.",
                    );
                }
                applied
            }
            GuiShellAction::AnnouncePublicServerSelectionChanged(index) => {
                self.announce_public_server_selection_changed(index)
            }
            GuiShellAction::BeginSavedServerConnect => self.begin_saved_server_connect(),
            GuiShellAction::CompleteSavedServerConnect => self.complete_saved_server_connect(),
            GuiShellAction::CancelSavedServerConnect => self.cancel_saved_server_connect(),
            GuiShellAction::BeginSessionDisconnect => self.begin_session_disconnect(),
            GuiShellAction::CompleteSessionDisconnect => self.complete_session_disconnect(),
            GuiShellAction::CancelSessionDisconnect => self.cancel_session_disconnect(),
            GuiShellAction::BeginSelectedPublicServerConnect => {
                self.begin_selected_public_server_connect()
            }
            GuiShellAction::CompleteSelectedPublicServerConnect => {
                self.complete_selected_public_server_connect()
            }
            GuiShellAction::BeginPublicServerRefresh => self.begin_public_server_refresh(),
            GuiShellAction::CompletePublicServerRefresh(servers) => {
                self.complete_public_server_refresh(servers)
            }
            GuiShellAction::AnnounceCustomPublicServerAdded { label, address } => {
                self.announce_custom_public_server_added(label, address)
            }
            GuiShellAction::SelectPublicServer(index) => {
                if !self.apply_public_server_selection(index) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AddMediaSearchDirectory(path) => {
                if !self.add_media_search_directory_path(path) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AnnounceMediaSearchDirectorySelected(index) => {
                self.announce_media_search_directory_selected(index)
            }
            GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(path) => {
                self.announce_media_search_directory_browsed(path)
            }
            GuiShellAction::BeginMissingMediaSearch => self.begin_missing_media_search(),
            GuiShellAction::CompleteMissingMediaSearch(found_path) => {
                self.complete_missing_media_search(found_path)
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
