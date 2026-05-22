use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_configuration_operation_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::BeginConfigurationSave => self.begin_configuration_save(),
            GuiShellAction::CompleteConfigurationSave(settings) => {
                self.complete_configuration_save(settings)
            }
            GuiShellAction::CancelConfigurationSave => self.cancel_configuration_save(),
            GuiShellAction::BeginConfigurationReset => self.begin_configuration_reset(),
            GuiShellAction::CompleteConfigurationReset(settings) => {
                self.complete_configuration_reset(settings)
            }
            GuiShellAction::CancelConfigurationReset => self.cancel_configuration_reset(),
            GuiShellAction::BeginConfigurationReload => self.begin_configuration_reload(),
            GuiShellAction::CompleteConfigurationReload(settings) => {
                self.complete_configuration_reload(settings)
            }
            GuiShellAction::CancelConfigurationReload => self.cancel_configuration_reload(),
            GuiShellAction::BeginClearGuiData => self.begin_clear_gui_data(),
            GuiShellAction::CompleteClearGuiData => self.complete_clear_gui_data(),
            GuiShellAction::CancelClearGuiData => self.cancel_clear_gui_data(),
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
