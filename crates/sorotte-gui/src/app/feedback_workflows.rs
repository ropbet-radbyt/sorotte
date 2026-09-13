use super::remote_services;
use super::shell_state::{
    GuiPendingOperationKind, GuiTransientNotificationLevel, MainWindowChatRow,
    SorotteGuiShellAppState,
};
use super::support::normalized_editable_text;
use super::ui_state::GuiUpdateCheckState;
use sorotte_secret::SecretValue;

impl SorotteGuiShellAppState {
    pub(super) fn append_chat_row(&mut self, sender: String, message: String) {
        self.main_window
            .chat
            .push(MainWindowChatRow { sender, message });
    }

    pub(super) fn begin_local_chat_send(&mut self, message: String) -> bool {
        if !self.commands.can_send_chat_message {
            return self.record_action_error(self.chat_send_unavailable_message());
        }
        if normalized_editable_text(&message).is_none() {
            return self.record_action_error("Local chat messages must be non-empty.");
        }

        self.outgoing_chat_message = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_local_chat_send(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            self.clear_action_error_and_refresh();
            return true;
        };
        if pending.kind != GuiPendingOperationKind::SendChatMessage {
            return self.record_action_error("No local chat send is currently in progress.");
        }
        self.outgoing_chat_message = None;
        self.pending_operation = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_local_chat_send(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No local chat send is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SendChatMessage {
            return self.record_action_error("No local chat send is currently in progress.");
        }
        if self.outgoing_chat_message.is_none() {
            self.pending_operation = None;
            return self.record_action_error("No local chat send is currently in progress.");
        }

        self.outgoing_chat_message = None;
        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Chat send canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_remote_chat_message(&mut self, sender: String, message: String) -> bool {
        let Some(sender) = normalized_editable_text(&sender) else {
            return self
                .record_action_error("Remote chat sender and message must both be non-empty.");
        };
        let Some(message) = normalized_editable_text(&message) else {
            return self
                .record_action_error("Remote chat sender and message must both be non-empty.");
        };

        self.append_chat_row(sender, message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_system_chat_event(&mut self, message: String) -> bool {
        let Some(message) = normalized_editable_text(&message) else {
            return self.record_action_error("System chat messages must be non-empty.");
        };
        self.push_system_chat_message(message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_controlled_room_created(
        &mut self,
        room: String,
        password: SecretValue,
    ) -> bool {
        let password = password.expose_secret();
        let share_code = format!("{room}:{password}");
        self.push_system_chat_message(format!(
            "Created controlled room {room} with password {password} ({share_code})."
        ));
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_check_language(&self) -> String {
        self.runtime_language_tag().to_owned()
    }

    pub(super) fn update_check_channel(&self) -> Option<String> {
        self.saved_configuration
            .update_channel
            .as_deref()
            .and_then(normalized_editable_text)
            .map(|value| value.to_ascii_lowercase())
    }

    pub(super) fn begin_update_check(&mut self, user_initiated: bool) -> bool {
        self.update_check.status = Some(remote_services::UpdateCheckStatus::Checking);
        self.update_check.message = Some("Checking for updates".to_owned());
        self.update_check.user_initiated = user_initiated;
        self.update_check.download_state = remote_services::UpdateDownloadState::Idle;
        self.update_check.staged_update = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_update_check_result(
        &mut self,
        result: remote_services::UpdateCheckResult,
    ) -> bool {
        let mut settings = self.configuration.to_stored_settings();
        settings.last_checked_for_updates = Some(result.checked_at_utc.clone());
        if let Some(public_servers) = result.public_servers.as_ref() {
            settings.public_servers = Some(public_servers.clone());
        }
        self.resync_from_settings(settings);
        self.update_check = GuiUpdateCheckState {
            status: Some(result.status.clone()),
            message: Some(result.message.clone()),
            url: result.url.clone(),
            candidate: result.candidate.clone(),
            download_state: remote_services::UpdateDownloadState::Idle,
            staged_update: None,
            self_update_supported: result.self_update_supported,
            last_checked_for_updates: Some(result.checked_at_utc.clone()),
            user_initiated: result.user_initiated,
        };
        if matches!(
            self.update_check.status_level(),
            GuiTransientNotificationLevel::Warning | GuiTransientNotificationLevel::Error
        ) {
            self.push_transient_notification(self.update_check.status_level(), result.message);
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn activate_update_indicator(&mut self) -> bool {
        if self
            .update_check
            .update_indicator_activation_action()
            .is_none()
        {
            return self.record_action_error("The update action is not available yet.");
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_update_download(&mut self) -> bool {
        if self.update_check.candidate.is_none() {
            return self.record_action_error("No update package is available to download.");
        }
        if !self.update_check.self_update_supported {
            return self.record_action_error(
                "This Sorotte GUI build is not a packaged install; self-update is disabled.",
            );
        }
        self.update_check.download_state = remote_services::UpdateDownloadState::Downloading;
        self.update_check.message = Some("Downloading and staging update...".to_owned());
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_update_download_result(
        &mut self,
        result: remote_services::UpdateDownloadResult,
    ) -> bool {
        self.update_check.download_state = result.state;
        self.update_check.message = Some(result.message.clone());
        self.update_check.staged_update = result.staged_update;
        if matches!(result.state, remote_services::UpdateDownloadState::Failed) {
            self.push_transient_notification(GuiTransientNotificationLevel::Error, result.message);
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_staged_update_apply(&mut self) -> bool {
        if self.update_check.staged_update.is_none() {
            return self.record_action_error("No staged update is ready to apply.");
        }
        self.update_check.message = Some("Launching update helper...".to_owned());
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_staged_update_launch_result(
        &mut self,
        result: remote_services::UpdateApplyLaunchResult,
    ) -> bool {
        self.update_check.message = Some(result.message.clone());
        self.push_transient_notification(
            if result.success {
                GuiTransientNotificationLevel::Info
            } else {
                GuiTransientNotificationLevel::Error
            },
            result.message,
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_startup_public_server_cache(
        &mut self,
        servers: Vec<(String, String)>,
    ) -> bool {
        let mut settings = self.configuration.to_stored_settings();
        settings.public_servers = Some(servers);
        self.resync_from_settings(settings);
        self.clear_action_error_and_refresh();
        true
    }
}
