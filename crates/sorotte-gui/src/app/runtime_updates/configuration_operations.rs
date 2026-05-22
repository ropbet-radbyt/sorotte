use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

use super::super::shell_state::{
    GuiConfigurationTab, GuiPendingOperationKind, GuiPendingOperationState,
    GuiRoomHistoryEditSessionState, GuiShellView, GuiTransientNotificationLevel,
    SorotteGuiShellAppState,
};

impl SorotteGuiShellAppState {
    pub(in crate::app) fn begin_room_history_edit(&mut self) -> bool {
        if self.room_history_edit_session.is_some() {
            return self.record_action_error("A room-history edit session is already active.");
        }
        self.room_history_edit_session = Some(GuiRoomHistoryEditSessionState {
            buffer: self.configuration.room_history_multiline_text(),
            is_dirty: false,
        });
        self.active_view = GuiShellView::Setup;
        self.select_configuration_tab(GuiConfigurationTab::Connection);
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn update_room_history_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.room_history_edit_session.as_mut() else {
            return self.record_action_error("No room-history edit session is currently active.");
        };
        let current_value = self.configuration.room_history_multiline_text();
        session.is_dirty = buffer != current_value;
        session.buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn commit_room_history_edit(&mut self) -> bool {
        let Some(session) = self.room_history_edit_session.clone() else {
            return self.record_action_error("No room-history edit session is currently active.");
        };
        self.configuration
            .apply_room_history_multiline_text(&session.buffer);
        self.room_history_edit_session = None;
        let room_history_count = self
            .configuration
            .settings
            .room_list
            .as_ref()
            .map_or(0, Vec::len);
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Room history updated: {room_history_count} entries."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_room_history_edit(&mut self) -> bool {
        if self.room_history_edit_session.is_none() {
            return self.record_action_error("No room-history edit session is currently active.");
        }
        self.room_history_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_configuration_save(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.validation.issues.is_empty() {
            return self.record_action_error(
                "Configuration cannot be saved while validation issues remain.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::SaveConfiguration,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_configuration_save(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration save is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SaveConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration save.");
        }

        self.saved_configuration = settings;
        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_configuration_reset(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.has_unsaved_configuration_changes() {
            return self.record_action_error(
                "Configuration reset is unavailable with no unsaved changes.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ResetConfiguration,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_configuration_reset(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reset is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ResetConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reset.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.resync_from_settings(settings.clone());
        self.saved_configuration = settings;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_configuration_reset(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reset is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ResetConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reset.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration reset canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_configuration_reload(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ReloadConfiguration,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_configuration_reload(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reload is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ReloadConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reload.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.resync_from_settings(settings.clone());
        self.saved_configuration = settings;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_clear_gui_data(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ClearGuiData,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_clear_gui_data(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No clear-GUI-data operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ClearGuiData {
            return self
                .record_action_error("The active GUI operation is not a clear-GUI-data request.");
        }

        self.reset_to_first_run_state(StoredClientSettingsMvp::default());
        self.pending_saved_server_connect_saves_configuration = false;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_clear_gui_data(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No clear-GUI-data operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ClearGuiData {
            return self
                .record_action_error("The active GUI operation is not a clear-GUI-data request.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Clear GUI data canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_configuration_reload(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reload is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ReloadConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reload.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration reload canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_configuration_save(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration save is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SaveConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration save.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration save canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
