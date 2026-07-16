use sorotte_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;

use super::shell_state::{
    GuiPendingOperationKind, GuiPendingOperationState, GuiSavedServerConnectIntent, GuiShellView,
    GuiTransientNotificationLevel, SettingId, SorotteGuiShellAppState,
};
use super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    pub(super) fn selected_public_server_index(&self) -> Option<usize> {
        self.public_servers
            .servers
            .iter()
            .position(|row| row.is_selected)
    }

    pub(super) fn selected_public_server_address(&self) -> Option<&str> {
        self.public_servers
            .servers
            .iter()
            .find(|row| row.is_selected)
            .map(|row| row.address.as_str())
    }

    pub(super) fn set_selected_public_server_index(&mut self, selected_index: Option<usize>) {
        for (index, row) in self.public_servers.servers.iter_mut().enumerate() {
            row.is_selected = selected_index == Some(index);
        }
    }

    pub(super) fn restore_selected_public_server_address(
        &mut self,
        selected_address: Option<&str>,
    ) {
        let Some(selected_address) = selected_address else {
            return;
        };
        let Some(selected_index) = self
            .public_servers
            .servers
            .iter()
            .position(|row| row.address == selected_address)
        else {
            return;
        };
        self.set_selected_public_server_index(Some(selected_index));
    }

    pub(super) fn apply_public_server_selection(&mut self, index: usize) -> bool {
        let Some(row) = self.public_servers.servers.get(index).cloned() else {
            return self.record_action_error("No public server exists at the requested index.");
        };
        self.set_selected_public_server_index(Some(index));

        let (host, port) =
            parse_host_and_optional_port_from_host_arg_legacy_compatible(&row.address);
        let _ = self
            .configuration
            .apply_text_value(SettingId::ConnectionHost, &host);
        let _ = self.configuration.apply_text_value(
            SettingId::ConnectionPort,
            &port.map_or_else(String::new, |value| value.to_string()),
        );
        true
    }

    pub(super) fn announce_public_server_selection_changed(&mut self, index: usize) -> bool {
        if !self.apply_public_server_selection(index) {
            return false;
        }
        let row = self.public_servers.servers[index].clone();
        self.push_system_chat_message(format!("Public server selected: {}.", row.label));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Public server selected: {}.", row.label),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_saved_server_connect(
        &mut self,
        intent: GuiSavedServerConnectIntent,
    ) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if let Some(message) = self.player_setup_connect_block_message() {
            return self.record_action_error(message);
        }
        if intent == GuiSavedServerConnectIntent::SaveAndConnect
            && !self.validation.issues.is_empty()
        {
            return self.record_action_error(
                "Save & connect is unavailable while validation issues remain.",
            );
        }
        if intent == GuiSavedServerConnectIntent::SaveAndConnect
            && self.pending_config_storage_target.is_some()
        {
            return self.record_action_error(
                "Save the pending config-location change before using Save & connect.",
            );
        }
        if !self.commands.can_connect_saved_server {
            return self.record_action_error(
                "Configured server connect requires a saved host and a valid port.",
            );
        }
        let Some(_target) = self.saved_session_connect_target() else {
            return self.record_action_error(
                "Configured server connect requires a saved host and a valid port.",
            );
        };
        self.pending_saved_server_connect_intent = Some(intent);
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ConnectSavedServer,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_saved_server_connect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No configured-server connect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ConnectSavedServer {
            return self
                .record_action_error("No configured-server connect is currently in progress.");
        }
        let Some(_target) = self.saved_session_connect_target() else {
            self.pending_operation = None;
            self.pending_saved_server_connect_intent = None;
            return self.record_action_error(
                "Configured server connect requires a saved host and a valid port.",
            );
        };
        self.pending_operation = None;
        self.pending_saved_server_connect_intent = None;
        self.active_view = GuiShellView::Room;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_saved_server_connect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No configured-server connect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ConnectSavedServer {
            return self
                .record_action_error("No configured-server connect is currently in progress.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_intent = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configured server connect canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_session_disconnect(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.commands.can_disconnect_session {
            return self.record_action_error(
                "Session disconnect is unavailable when no session runtime is active.",
            );
        }
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::DisconnectSession,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_session_disconnect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No session disconnect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::DisconnectSession {
            return self.record_action_error("No session disconnect is currently in progress.");
        }
        self.pending_operation = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_session_disconnect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No session disconnect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::DisconnectSession {
            return self.record_action_error("No session disconnect is currently in progress.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Session disconnect canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_selected_public_server_connect(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.commands.can_connect_public_server {
            return self.record_action_error(
                "Public server connect is unavailable when browser connect actions are disabled.",
            );
        }
        let Some(index) = self.selected_public_server_index() else {
            return self.record_action_error("No public server is currently selected.");
        };
        if !self.apply_public_server_selection(index) {
            return false;
        }
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ConnectPublicServer,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_selected_public_server_connect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No public server connect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ConnectPublicServer {
            return self.record_action_error("No public server connect is currently in progress.");
        }
        let Some(index) = self.selected_public_server_index() else {
            self.pending_operation = None;
            return self.record_action_error("No public server is currently selected.");
        };
        let Some(_row) = self.public_servers.servers.get(index).cloned() else {
            self.pending_operation = None;
            return self.record_action_error("No public server exists at the requested index.");
        };
        self.pending_operation = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_selected_public_server_connect(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No public server connect is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ConnectPublicServer {
            return self.record_action_error("No public server connect is currently in progress.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Public server connect canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_public_server_refresh(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.commands.can_refresh_public_servers {
            return self.record_action_error(
                "Public server refresh is unavailable when browser refresh actions are disabled.",
            );
        }
        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::RefreshPublicServers,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_public_server_refresh(
        &mut self,
        servers: Vec<(String, String)>,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No public server refresh is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::RefreshPublicServers {
            return self.record_action_error("No public server refresh is currently in progress.");
        }

        let mut normalized = Vec::new();
        for (label, address) in servers {
            let Some(label) = normalized_editable_text(&label) else {
                continue;
            };
            let Some(address) = normalized_editable_text(&address) else {
                continue;
            };
            let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
            if host.trim().is_empty() {
                continue;
            }
            normalized.push((label, address));
        }

        let mut settings = self.configuration.to_stored_settings();
        settings.public_servers = if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        };
        self.resync_from_settings(settings);
        self.pending_operation = None;
        if self.public_servers.servers.is_empty() {
            self.set_selected_public_server_index(None);
            self.push_transient_notification(
                GuiTransientNotificationLevel::Warning,
                "Public servers refreshed: none available.".to_owned(),
            );
        } else {
            self.set_selected_public_server_index(Some(0));
            let _ = self.apply_public_server_selection(0);
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_public_server_refresh(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No public server refresh is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::RefreshPublicServers {
            return self.record_action_error("No public server refresh is currently in progress.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Public server refresh canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_custom_public_server_added(
        &mut self,
        label: String,
        address: String,
    ) -> bool {
        let Some(label) = normalized_editable_text(&label) else {
            return self.record_action_error(
                "Custom public-server label and address must both be non-empty.",
            );
        };
        let Some(address) = normalized_editable_text(&address) else {
            return self.record_action_error(
                "Custom public-server label and address must both be non-empty.",
            );
        };
        let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
        if host.trim().is_empty() {
            return self.record_action_error("Custom public-server address is not valid.");
        }

        let mut settings = self.configuration.to_stored_settings();
        let mut servers = settings.public_servers.take().unwrap_or_default();
        servers.push((label.clone(), address));
        let selected_index = servers.len() - 1;
        settings.public_servers = Some(servers);
        self.resync_from_settings(settings);
        self.set_selected_public_server_index(Some(selected_index));
        let _ = self.apply_public_server_selection(selected_index);
        self.push_system_chat_message(format!("Custom public server added: {label}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Custom public server added: {label}."),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
