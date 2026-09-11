use super::GuiRuntimeState;
use crate::app::shell_state::*;
use crate::app::support::normalized_editable_text;
use crate::app::ui_state::GuiUpdateCheckState;
use crate::app::{configuration_model, remote_services};
use sorotte_client_app::app_boundary::state::StoredClientSettings;

impl GuiRuntimeState {
    pub(super) fn refresh_update_policy(&mut self) {
        self.updates.policy.automatic =
            self.settings.saved.check_for_updates_automatically == Some(true);
        self.updates.policy.last_checked_for_updates =
            self.settings.saved.last_checked_for_updates.clone();
        self.updates.policy.language = self.runtime_language_tag().to_owned();
        self.updates.policy.channel = self
            .settings
            .saved
            .update_channel
            .as_deref()
            .and_then(normalized_editable_text)
            .map(|value| value.to_ascii_lowercase());
    }
    pub(super) fn apply_service_output(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::ApplyStartupPublicServerCache(servers) => {
                let mut settings = self.settings.draft.to_stored_settings();
                settings.public_servers = Some(servers);
                self.resync_from_settings(settings);
            }
            GuiShellAction::CompletePublicServerRefresh(servers) => {
                if !self.pending_operation_is(GuiPendingOperationKind::RefreshPublicServers) {
                    return self
                        .record_action_error("No public server refresh is currently in progress.");
                }
                let mut settings = self.settings.draft.to_stored_settings();
                settings.public_servers =
                    Some(configuration_model::normalize_public_servers(servers));
                self.resync_from_settings(settings);
                self.session.pending_operation = None;
                if self.session.public_servers.servers.is_empty() {
                    configuration_model::set_selected_public_server_index(
                        &mut self.session.public_servers,
                        None,
                    );
                } else {
                    let index = self
                        .session
                        .public_servers
                        .servers
                        .iter()
                        .position(|row| row.is_selected)
                        .unwrap_or(0);
                    if let Err(message) = configuration_model::apply_public_server_selection(
                        &mut self.session.public_servers,
                        &mut self.settings.draft,
                        index,
                    ) {
                        return self.record_action_error(message);
                    }
                }
            }
            GuiShellAction::CompleteMissingMediaSearch(path) => {
                if !self.pending_operation_is(GuiPendingOperationKind::SearchMissingMedia) {
                    return self
                        .record_action_error("No missing-media search is currently in progress.");
                }
                self.session.pending_operation = None;
                if path.as_deref().and_then(normalized_editable_text).is_none() {
                    self.push_system_chat_message(
                        "Missing media search completed: no match found.".to_owned(),
                    );
                }
            }
            GuiShellAction::CompleteClearGuiData => {
                if !self.pending_operation_is(GuiPendingOperationKind::ClearGuiData) {
                    return false;
                }
                *self = Self::from_stored_settings(&StoredClientSettings::default());
            }
            GuiShellAction::CancelPendingOperation => {
                let Some(kind) = self
                    .session
                    .pending_operation
                    .as_ref()
                    .map(|pending| pending.kind)
                else {
                    return false;
                };
                self.cancel_operation(kind);
            }
            GuiShellAction::CancelConfigurationSave => {
                if !self.cancel_operation(GuiPendingOperationKind::SaveConfiguration) {
                    return false;
                }
            }
            GuiShellAction::CancelDiscardConfigurationChanges => {
                if !self.cancel_operation(GuiPendingOperationKind::DiscardConfigurationChanges) {
                    return false;
                }
            }
            GuiShellAction::CancelConfigurationReload => {
                if !self.cancel_operation(GuiPendingOperationKind::ReloadConfiguration) {
                    return false;
                }
            }
            GuiShellAction::CancelClearGuiData => {
                if !self.cancel_operation(GuiPendingOperationKind::ClearGuiData) {
                    return false;
                }
            }
            GuiShellAction::CancelConfigStorageRootChange => {
                if !self.cancel_operation(GuiPendingOperationKind::ChangeConfigStorageRoot) {
                    return false;
                }
            }
            GuiShellAction::CancelSavedServerConnect => {
                if !self.cancel_operation(GuiPendingOperationKind::ConnectSavedServer) {
                    return false;
                }
            }
            GuiShellAction::CancelSessionDisconnect => {
                if !self.cancel_operation(GuiPendingOperationKind::DisconnectSession) {
                    return false;
                }
            }
            GuiShellAction::CancelLocalChatSend => {
                if !self.cancel_operation(GuiPendingOperationKind::SendChatMessage) {
                    return false;
                }
            }
            GuiShellAction::CancelPlaybackPauseState => {
                use GuiPendingOperationKind::{SetPlaybackPause, TogglePlaybackPause};
                let Some(kind @ (SetPlaybackPause(_) | TogglePlaybackPause)) = self
                    .session
                    .pending_operation
                    .as_ref()
                    .map(|pending| pending.kind)
                else {
                    return false;
                };
                self.cancel_operation(kind);
            }
            GuiShellAction::CancelPlaybackPauseToggle => {
                if !self.cancel_operation(GuiPendingOperationKind::TogglePlaybackPause) {
                    return false;
                }
            }
            GuiShellAction::EditConfigurationText { id, value } => {
                let previous_settings = self.settings.draft.to_stored_settings();
                if !self
                    .settings
                    .draft
                    .apply_text_value(id, value.expose_for_config_apply())
                {
                    return self
                        .record_action_error("Configuration text control could not be updated.");
                }
                self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
            }
            GuiShellAction::BeginUpdateCheck { user_initiated } => {
                self.updates.model.status = Some(remote_services::UpdateCheckStatus::Checking);
                self.updates.model.message = Some("Checking for updates".to_owned());
                self.updates.model.user_initiated = user_initiated;
                self.updates.model.download_state = remote_services::UpdateDownloadState::Idle;
                self.updates.model.staged_update = None;
            }
            GuiShellAction::ApplyUpdateCheckResult(result) => {
                let mut settings = self.settings.draft.to_stored_settings();
                settings.last_checked_for_updates = Some(result.checked_at_utc.clone());
                if let Some(servers) = result.public_servers.as_ref() {
                    settings.public_servers = Some(servers.clone());
                }
                self.resync_from_settings(settings);
                self.updates.model = GuiUpdateCheckState {
                    status: Some(result.status),
                    message: Some(result.message),
                    url: result.url,
                    candidate: result.candidate,
                    download_state: remote_services::UpdateDownloadState::Idle,
                    staged_update: None,
                    self_update_supported: result.self_update_supported,
                    last_checked_for_updates: Some(result.checked_at_utc),
                    user_initiated: result.user_initiated,
                };
                self.session.menus.update_notice_expected = false;
            }
            GuiShellAction::BeginUpdateDownload | GuiShellAction::BeginUpdateInstall => {
                if self.updates.model.candidate.is_none() {
                    return self.record_action_error("No update package is available to download.");
                }
                if !self.updates.model.self_update_supported {
                    return self.record_action_error("This Sorotte GUI build is not a packaged install; self-update is disabled.");
                }
                self.updates.model.download_state =
                    remote_services::UpdateDownloadState::Downloading;
                self.updates.model.message = Some("Downloading and staging update...".to_owned());
            }
            GuiShellAction::ApplyUpdateDownloadResult(result) => {
                self.updates.model.download_state = result.state;
                self.updates.model.message = Some(result.message);
                self.updates.model.staged_update = result.staged_update;
            }
            GuiShellAction::BeginStagedUpdateApply => {
                if self.updates.model.staged_update.is_none() {
                    return self.record_action_error("No staged update is ready to apply.");
                }
                self.updates.model.message = Some("Launching update helper...".to_owned());
            }
            GuiShellAction::ApplyStagedUpdateLaunchResult(result) => {
                self.updates.model.message = Some(result.message);
            }
            // These outputs only affect UI-owned presentation or apply requirements.
            GuiShellAction::ApplyPendingApplyRequirementsSnapshot(_)
            | GuiShellAction::PushTransientNotification { .. }
            | GuiShellAction::OpenModal(_)
            | GuiShellAction::CloseModal
            | GuiShellAction::SwitchView(_) => return false,
            // User input is applied by the UI before it submits its feature state.
            _ => return false,
        }
        self.refresh_update_policy();
        self.clear_action_error_and_refresh();
        true
    }

    fn cancel_operation(&mut self, kind: GuiPendingOperationKind) -> bool {
        if !self.pending_operation_is(kind) {
            return false;
        }
        self.session.pending_operation = None;
        match kind {
            GuiPendingOperationKind::ChangeConfigStorageRoot => {
                self.settings.pending_storage_target = None;
                self.session.pending_saved_server_connect_intent = None;
            }
            GuiPendingOperationKind::SaveConfiguration
            | GuiPendingOperationKind::ReloadConfiguration
            | GuiPendingOperationKind::ClearGuiData
            | GuiPendingOperationKind::ConnectSavedServer => {
                self.session.pending_saved_server_connect_intent = None;
            }
            GuiPendingOperationKind::SendChatMessage => {
                self.session.outgoing_chat_message = None;
            }
            _ => {}
        }
        true
    }
}
