use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn append_chat_row(&mut self, sender: String, message: String) {
        self.main_window
            .chat
            .push(MainWindowChatRow { sender, message });
    }

    pub(super) fn begin_local_chat_send(&mut self, message: String) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.commands.can_send_chat_message {
            return self.record_action_error(
                "Local chat sending is unavailable when chat input is disabled.",
            );
        }
        let Some(message) = normalized_editable_text(&message) else {
            return self.record_action_error("Local chat messages must be non-empty.");
        };

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::SendChatMessage,
        });
        self.outgoing_chat_message = Some(message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_local_chat_send(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No local chat send is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SendChatMessage {
            return self.record_action_error("No local chat send is currently in progress.");
        }
        let Some(message) = self.outgoing_chat_message.take() else {
            self.pending_operation = None;
            return self.record_action_error("No local chat send is currently in progress.");
        };
        let sender = self
            .main_window
            .users
            .iter()
            .find(|user| user.is_self)
            .map(|user| user.username.clone())
            .unwrap_or_else(|| "You".to_owned());

        self.append_chat_row(sender, message);
        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Chat sent.".to_owned(),
        );
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

    pub(super) fn announce_tls_certificate_prompt_required(&mut self) -> bool {
        self.menus.tls_prompt_expected = true;
        self.open_modal = Some(GuiShellModal::TlsCertificatePrompt);
        self.push_system_chat_message("TLS certificate prompt opened.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "TLS certificate prompt opened.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_update_notice_available(&mut self) -> bool {
        self.menus.update_notice_expected = true;
        self.open_modal = Some(GuiShellModal::UpdateNotice);
        if self.update_check.message.is_none() {
            self.update_check.message =
                Some("An update notice is available for this client build.".to_owned());
        }
        self.push_system_chat_message("Update notice opened.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Update notice opened.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_check_language(&self) -> String {
        self.configuration
            .to_stored_settings()
            .language
            .as_deref()
            .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
            .unwrap_or("en")
            .to_owned()
    }

    pub(super) fn begin_update_check(&mut self, user_initiated: bool) -> bool {
        let language = self.update_check_language();
        let result = remote_services::check_for_updates(Some(language.as_str()), user_initiated);
        self.apply_update_check_result(result)
    }

    pub(super) fn apply_update_check_result(
        &mut self,
        result: remote_services::LegacyUpdateCheckResult,
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
            last_checked_for_updates: Some(result.checked_at_utc.clone()),
            user_initiated: result.user_initiated,
        };
        self.menus.update_notice_expected = self.update_check.should_open_modal();
        self.open_modal = self
            .update_check
            .should_open_modal()
            .then_some(GuiShellModal::UpdateNotice);
        self.push_system_chat_message(result.message.clone());
        self.push_transient_notification(self.update_check.status_level(), result.message);
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

    pub(super) fn dismiss_update_notice(&mut self) -> bool {
        let had_notice = self.menus.update_notice_expected
            || self.open_modal == Some(GuiShellModal::UpdateNotice);
        if !had_notice {
            return self.record_action_error("No update notice is currently active.");
        }

        self.menus.update_notice_expected = false;
        if self.open_modal == Some(GuiShellModal::UpdateNotice) {
            self.open_modal = None;
        }
        self.push_system_chat_message("Update notice dismissed.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Update notice dismissed.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_tls_certificate_prompt(&mut self, trusted: bool) -> bool {
        let had_prompt = self.menus.tls_prompt_expected
            || self.open_modal == Some(GuiShellModal::TlsCertificatePrompt);
        if !had_prompt {
            return self.record_action_error("No TLS certificate prompt is currently active.");
        }

        self.menus.tls_prompt_expected = false;
        if self.open_modal == Some(GuiShellModal::TlsCertificatePrompt) {
            self.open_modal = None;
        }
        let message = if trusted {
            "TLS certificate trusted for this session."
        } else {
            "TLS certificate rejected."
        };
        self.push_system_chat_message(message.to_owned());
        self.push_transient_notification(
            if trusted {
                GuiTransientNotificationLevel::Success
            } else {
                GuiTransientNotificationLevel::Warning
            },
            message.to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
