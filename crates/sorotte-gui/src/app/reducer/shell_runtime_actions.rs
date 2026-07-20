use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_shell_runtime_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::SwitchView(view) => {
                self.active_view = view;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SelectConfigurationTab(tab) => {
                self.select_configuration_tab(tab);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::OpenModal(modal) => {
                self.open_modal = Some(modal);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CloseModal => self.close_modal_window(),
            GuiShellAction::DismissUpdateNotice => self.dismiss_update_notice(),
            GuiShellAction::BeginUpdateCheck { user_initiated } => {
                self.begin_update_check(user_initiated)
            }
            GuiShellAction::ApplyUpdateCheckResult(result) => {
                self.apply_update_check_result(result)
            }
            GuiShellAction::ActivateUpdateIndicator => self.activate_update_indicator(),
            GuiShellAction::BeginUpdateDownload => self.begin_update_download(),
            GuiShellAction::BeginUpdateInstall => self.begin_update_download(),
            GuiShellAction::ApplyUpdateDownloadResult(result) => {
                self.apply_update_download_result(result)
            }
            GuiShellAction::BeginStagedUpdateApply => self.begin_staged_update_apply(),
            GuiShellAction::ApplyStagedUpdateLaunchResult(result) => {
                self.apply_staged_update_launch_result(result)
            }
            GuiShellAction::ApplyStartupPublicServerCache(servers) => {
                self.apply_startup_public_server_cache(servers)
            }
            GuiShellAction::TrustTlsCertificatePrompt => self.complete_tls_certificate_prompt(true),
            GuiShellAction::RejectTlsCertificatePrompt => {
                self.complete_tls_certificate_prompt(false)
            }
            GuiShellAction::TriggerSelectedMenuAction => self.trigger_selected_menu_action(),
            GuiShellAction::InvokeMenuAction(action_id) => self.invoke_menu_action(action_id),
            GuiShellAction::AnnounceTlsCertificatePromptRequired => {
                self.announce_tls_certificate_prompt_required()
            }
            GuiShellAction::AnnounceUpdateNoticeAvailable => {
                self.announce_update_notice_available()
            }
            GuiShellAction::AnnounceAboutDialogRequested => self.announce_about_dialog_requested(),
            GuiShellAction::AnnounceHelpRequested => self.announce_help_requested(),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot) => {
                self.apply_menu_dialog_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(snapshot) => {
                self.apply_gui_feedback_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiErrorRuntimeSnapshot(snapshot) => {
                self.apply_gui_error_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(snapshot) => {
                self.apply_gui_command_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(snapshot) => {
                self.apply_gui_media_index_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(snapshot) => {
                self.apply_gui_player_setup_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(snapshot) => {
                self.apply_gui_seek_preparation_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(snapshot) => {
                self.apply_gui_stream_helper_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(snapshot) => {
                self.apply_gui_stream_helper_remediation_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot) => {
                self.apply_gui_media_match_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(snapshot) => {
                self.apply_gui_media_match_remediation_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiPlexRuntimeSnapshot(snapshot) => {
                self.apply_gui_plex_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot) => {
                self.apply_gui_interaction_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(snapshot) => {
                self.apply_gui_draft_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(snapshot) => {
                self.apply_gui_configuration_draft_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(snapshot) => {
                self.apply_gui_saved_configuration_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiPersistedSettingsPatch(patch) => {
                self.apply_gui_persisted_settings_patch(patch)
            }
            GuiShellAction::ApplyPendingApplyRequirementsSnapshot(requirements) => {
                self.replace_pending_apply_requirements(requirements);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(snapshot) => {
                self.apply_gui_configuration_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(snapshot) => {
                self.apply_gui_config_storage_runtime_snapshot(snapshot)
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
