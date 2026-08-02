use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

use super::super::shell_state::{
    GuiConfigStorageChangeTarget, GuiConfigStorageRuntimeSnapshot, GuiConfigurationTab,
    GuiPendingOperationKind, GuiPendingOperationState, GuiRoomHistoryEditSessionState,
    GuiSettingApplyRequirement, GuiShellView, GuiTransientNotificationLevel,
    SorotteGuiShellAppState,
};
use super::super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    pub(in crate::app) fn apply_synchronization_profile(
        &mut self,
        profile: super::super::shell_state::SynchronizationProfileId,
    ) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if self.text_edit_session.is_some() {
            return self.record_action_error(
                "Finish the active setting edit before applying a synchronization profile.",
            );
        }

        let current_settings = self.configuration.to_stored_settings();
        if profile.matches(&current_settings) {
            return self.record_action_error(format!(
                "{} is already the active synchronization profile.",
                profile.label()
            ));
        }

        self.configuration.apply_synchronization_profile(profile);
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!(
                "{} applied to the draft. Save changes to keep it.",
                profile.label()
            ),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn replace_pending_apply_requirements(
        &mut self,
        requirements: impl IntoIterator<Item = GuiSettingApplyRequirement>,
    ) {
        self.pending_apply_requirements = requirements
            .into_iter()
            .filter(|requirement| {
                !matches!(
                    requirement,
                    GuiSettingApplyRequirement::Immediate | GuiSettingApplyRequirement::OnSave
                )
            })
            .collect();
        self.pending_apply_requirements.sort_unstable();
        self.pending_apply_requirements.dedup();
    }

    pub(in crate::app) fn settle_persisted_configuration(
        &mut self,
        settings: StoredClientSettingsMvp,
        preserve_pending_storage_target: bool,
    ) {
        let pending_storage_target = preserve_pending_storage_target
            .then(|| self.pending_config_storage_target.clone())
            .flatten();
        self.resync_from_settings(settings.clone());
        self.saved_configuration = settings;
        self.configuration.settings.server_password =
            self.saved_configuration.server_password.clone();
        self.configuration.server_password = super::super::shell_state::SecretDraft::Unchanged;
        self.text_edit_session = None;
        self.room_history_edit_session = None;
        self.focused_configuration_control = None;
        if preserve_pending_storage_target {
            self.pending_config_storage_target = pending_storage_target;
        }
    }

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
        if !self.has_unsaved_configuration_changes() {
            return self
                .record_action_error("Save changes is unavailable with no unsaved changes.");
        }
        if !self.validation.issues.is_empty() {
            return self.record_action_error(
                "Configuration cannot be saved while validation issues remain.",
            );
        }

        let pending_kind = if self.pending_config_storage_target.is_some() {
            GuiPendingOperationKind::ChangeConfigStorageRoot
        } else {
            GuiPendingOperationKind::SaveConfiguration
        };
        self.pending_operation = Some(GuiPendingOperationState { kind: pending_kind });
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

        self.settle_persisted_configuration(settings, false);
        self.pending_operation = None;
        self.pending_config_storage_target = None;
        self.pending_saved_server_connect_intent = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Configuration saved.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_discard_configuration_changes(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.has_unsaved_configuration_changes() {
            return self
                .record_action_error("Discard changes is unavailable with no unsaved changes.");
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::DiscardConfigurationChanges,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_discard_configuration_changes(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No discard-changes operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::DiscardConfigurationChanges {
            return self.record_action_error("The active GUI operation is not discard changes.");
        }

        self.settle_persisted_configuration(settings, false);
        self.pending_operation = None;
        self.pending_config_storage_target = None;
        self.pending_saved_server_connect_intent = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_discard_configuration_changes(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No discard-changes operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::DiscardConfigurationChanges {
            return self.record_action_error("The active GUI operation is not discard changes.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Discard changes canceled.".to_owned(),
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

        self.settle_persisted_configuration(settings, false);
        self.pending_operation = None;
        self.pending_config_storage_target = None;
        self.pending_saved_server_connect_intent = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_clear_gui_data(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if self.clear_gui_data_confirmation_visible {
            return self.record_action_error("Clear-GUI-data confirmation is already open.");
        }

        self.clear_gui_data_confirmation_visible = true;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn confirm_clear_gui_data(&mut self) -> bool {
        if !self.clear_gui_data_confirmation_visible {
            return self.record_action_error("No clear-GUI-data confirmation is currently open.");
        }
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }

        self.clear_gui_data_confirmation_visible = false;
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ClearGuiData,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn dismiss_clear_gui_data_confirmation(&mut self) -> bool {
        if !self.clear_gui_data_confirmation_visible {
            return self.record_action_error("No clear-GUI-data confirmation is currently open.");
        }

        self.clear_gui_data_confirmation_visible = false;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_config_storage_root_change(&mut self, root: String) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if self.config_storage.external_override_active {
            return self.record_action_error(
                "The config location is controlled by a CLI or environment override.",
            );
        }
        let Some(root) = normalized_editable_text(&root) else {
            return self.record_action_error("Choose a non-empty config folder.");
        };
        self.pending_config_storage_target = Some(GuiConfigStorageChangeTarget::CustomRoot(root));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Config location selected. Save configuration to apply it.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn begin_config_storage_default_reset(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if self.config_storage.external_override_active {
            return self.record_action_error(
                "The config location is controlled by a CLI or environment override.",
            );
        }
        self.pending_config_storage_target = Some(GuiConfigStorageChangeTarget::DefaultRoot);
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Default config location selected. Save configuration to apply it.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn complete_config_storage_root_change(
        &mut self,
        snapshot: GuiConfigStorageRuntimeSnapshot,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No config-location change is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ChangeConfigStorageRoot {
            return self
                .record_action_error("The active GUI operation is not a config-location change.");
        }

        self.settle_persisted_configuration(settings, false);
        self.config_storage = snapshot;
        self.pending_operation = None;
        self.pending_config_storage_target = None;
        self.pending_saved_server_connect_intent = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn cancel_config_storage_root_change(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No config-location change is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ChangeConfigStorageRoot {
            return self
                .record_action_error("The active GUI operation is not a config-location change.");
        }

        self.pending_operation = None;
        self.pending_config_storage_target = None;
        self.pending_saved_server_connect_intent = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Config location change canceled.".to_owned(),
        );
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
        self.pending_saved_server_connect_intent = None;
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
        self.clear_gui_data_confirmation_visible = false;
        self.pending_saved_server_connect_intent = None;
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
        self.pending_saved_server_connect_intent = None;
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
        self.pending_saved_server_connect_intent = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration save canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
