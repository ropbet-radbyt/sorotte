use super::shell_state::{
    GuiPendingOperationKind, GuiPendingOperationState, GuiTransientNotificationLevel,
    SyncplayGuiShellAppState,
};
use super::support::normalized_editable_text;

impl SyncplayGuiShellAppState {
    pub(super) fn add_media_search_directory_path(&mut self, path: String) -> bool {
        let Some(path) = normalized_editable_text(&path) else {
            return self.record_action_error("Media search directory cannot be empty.");
        };
        let mut settings = self.configuration.to_stored_settings();
        let mut directories = settings.media_search_directories.take().unwrap_or_default();
        if directories.iter().any(|existing| existing == &path) {
            return self.record_action_error("Media search directory is already present.");
        }
        directories.push(path);
        settings.media_search_directories = Some(directories);
        self.resync_from_settings(settings);
        self.selection.selected_media_search_directory =
            self.media_search.directories.len().checked_sub(1);
        self.apply_selection_to_surfaces();
        true
    }

    pub(super) fn announce_media_search_directory_selected(&mut self, index: usize) -> bool {
        if index >= self.media_search.directories.len() {
            return self
                .record_action_error("No media-search directory exists at the requested index.");
        }
        self.selection.selected_media_search_directory = Some(index);
        self.apply_selection_to_surfaces();
        let path = self.media_search.directories[index].path.clone();
        self.push_system_chat_message(format!("Media search directory selected: {path}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Media search directory selected: {path}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_media_search_directory_browsed(&mut self, path: String) -> bool {
        if !self.add_media_search_directory_path(path) {
            return false;
        }
        let Some(index) = self.selection.selected_media_search_directory else {
            return self
                .record_action_error("The browsed media-search directory could not be selected.");
        };
        let path = self.media_search.directories[index].path.clone();
        self.last_media_dialog_directory = Some(path.clone());
        self.push_system_chat_message(format!("Media search directory added: {path}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Media search directory added: {path}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_missing_media_search(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.commands.can_search_missing_media {
            return self.record_action_error(
                "Missing-media search is unavailable when search actions are disabled.",
            );
        }
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::SearchMissingMedia,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_missing_media_search(&mut self, found_path: Option<String>) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No missing-media search is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SearchMissingMedia {
            return self.record_action_error("No missing-media search is currently in progress.");
        }
        self.pending_operation = None;

        let found_path = found_path.and_then(|path| normalized_editable_text(&path));
        match found_path {
            Some(_) => {}
            None => {
                self.push_system_chat_message(
                    "Missing media search completed: no match found.".to_owned(),
                );
                self.push_transient_notification(
                    GuiTransientNotificationLevel::Warning,
                    "Missing media search completed: no match found.".to_owned(),
                );
            }
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn toggle_main_window_hide_empty_rooms(&mut self) -> bool {
        self.main_window.hide_empty_rooms = !self.main_window.hide_empty_rooms;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn add_trusted_domain(&mut self, domain: String) -> bool {
        let Some(domain) = normalized_editable_text(&domain) else {
            return self.record_action_error("Trusted domain cannot be empty.");
        };

        let mut settings = self.configuration.to_stored_settings();
        let already_present = settings.trusted_domains.as_ref().is_some_and(|domains| {
            domains
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(&domain))
        });
        if already_present {
            self.push_transient_notification(
                GuiTransientNotificationLevel::Info,
                format!("Trusted domain already present: {domain}."),
            );
            self.clear_action_error_and_refresh();
            return true;
        }

        let mut trusted_domains = settings.trusted_domains.take().unwrap_or_default();
        trusted_domains.push(domain.clone());
        settings.trusted_domains = Some(trusted_domains);
        self.resync_from_settings(settings);
        self.push_system_chat_message(format!("Trusted domain added: {domain}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Trusted domain added: {domain}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_missing_media_search(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No missing-media search is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SearchMissingMedia {
            return self.record_action_error("No missing-media search is currently in progress.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Missing-media search canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
