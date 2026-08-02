use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_configuration_operation_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::ApplySynchronizationProfile(profile) => {
                self.apply_synchronization_profile(profile)
            }
            GuiShellAction::BeginConfigurationSave => self.begin_configuration_save(),
            GuiShellAction::CompleteConfigurationSave(settings) => {
                self.complete_configuration_save(settings)
            }
            GuiShellAction::CancelConfigurationSave => self.cancel_configuration_save(),
            GuiShellAction::BeginDiscardConfigurationChanges => {
                self.begin_discard_configuration_changes()
            }
            GuiShellAction::CompleteDiscardConfigurationChanges(settings) => {
                self.complete_discard_configuration_changes(settings)
            }
            GuiShellAction::CancelDiscardConfigurationChanges => {
                self.cancel_discard_configuration_changes()
            }
            GuiShellAction::BeginConfigurationReload => self.begin_configuration_reload(),
            GuiShellAction::CompleteConfigurationReload(settings) => {
                self.complete_configuration_reload(settings)
            }
            GuiShellAction::CancelConfigurationReload => self.cancel_configuration_reload(),
            GuiShellAction::BeginClearGuiData => self.begin_clear_gui_data(),
            GuiShellAction::ConfirmClearGuiData => self.confirm_clear_gui_data(),
            GuiShellAction::DismissClearGuiDataConfirmation => {
                self.dismiss_clear_gui_data_confirmation()
            }
            GuiShellAction::CompleteClearGuiData => self.complete_clear_gui_data(),
            GuiShellAction::CancelClearGuiData => self.cancel_clear_gui_data(),
            GuiShellAction::BeginConfigStorageRootChange(root) => {
                self.begin_config_storage_root_change(root)
            }
            GuiShellAction::BeginConfigStorageDefaultReset => {
                self.begin_config_storage_default_reset()
            }
            GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings } => {
                self.complete_config_storage_root_change(snapshot, settings)
            }
            GuiShellAction::CancelConfigStorageRootChange => {
                self.cancel_config_storage_root_change()
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
