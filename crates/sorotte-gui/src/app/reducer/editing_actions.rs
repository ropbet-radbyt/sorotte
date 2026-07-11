use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_editing_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::BeginPendingOperation(kind) => {
                if self.pending_operation.is_some() {
                    return self
                        .record_action_error("Another GUI operation is already in progress.");
                }
                self.pending_saved_server_connect_saves_configuration = false;
                self.pending_operation = Some(GuiPendingOperationState { kind });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CompletePendingOperation => {
                let Some(pending) = self.pending_operation.as_ref() else {
                    return self.record_action_error("No GUI operation is currently in progress.");
                };
                if pending.kind == GuiPendingOperationKind::SendChatMessage {
                    self.outgoing_chat_message = None;
                }
                self.pending_operation = None;
                self.pending_saved_server_connect_saves_configuration = false;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelPendingOperation => self.cancel_pending_operation(),
            GuiShellAction::FocusConfigurationControl { section, label } => {
                let Some(control) = self.configuration.control(section, label) else {
                    return self.record_action_error(
                        "No editable configuration control exists at the requested location.",
                    );
                };
                if !control.kind.is_editable() {
                    return self.record_action_error(
                        "The requested configuration control is not focusable.",
                    );
                }
                let activation_count = self
                    .focused_configuration_control
                    .as_ref()
                    .filter(|focused| focused.section == section && focused.label == label)
                    .map_or(0, |focused| focused.activation_count);
                self.focused_configuration_control = Some(GuiFocusedConfigurationControlState {
                    section,
                    label,
                    kind: control.kind,
                    activation_count,
                });
                if let Some(tab) = Self::configuration_tab_for_section(section) {
                    self.select_configuration_tab(tab);
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ActivateFocusedConfigurationControl => {
                let Some(focused) = self.focused_configuration_control.clone() else {
                    return self
                        .record_action_error("No configuration control is currently focused.");
                };

                if focused.kind == GuiDialogControlKind::Checkbox {
                    let Some(current_value) = self
                        .configuration
                        .control_value(focused.section, focused.label)
                    else {
                        return self.record_action_error(
                            "The focused configuration control no longer exists.",
                        );
                    };
                    let next_value = current_value != "yes";
                    let previous_settings = self.configuration.to_stored_settings();
                    let applied = self.configuration.apply_bool_value(
                        focused.section,
                        focused.label,
                        next_value,
                    );
                    if !applied {
                        return self.record_action_error(
                            "The focused checkbox control could not be toggled.",
                        );
                    }
                    if let Some(focused_state) = self.focused_configuration_control.as_mut() {
                        focused_state.activation_count += 1;
                    }
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                    return true;
                }

                let Some(control) = self.configuration.control(focused.section, focused.label)
                else {
                    return self.record_action_error(
                        "The focused configuration control no longer exists.",
                    );
                };
                self.text_edit_session = Some(GuiTextEditSessionState {
                    section: focused.section,
                    label: focused.label,
                    buffer: GuiConfigurationTextValue::for_control(
                        control.kind,
                        control.value.clone(),
                    ),
                    is_dirty: false,
                });
                if let Some(focused_state) = self.focused_configuration_control.as_mut() {
                    focused_state.activation_count += 1;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ClearConfigurationControlFocus => {
                let had_focus = self.focused_configuration_control.is_some();
                self.focused_configuration_control = None;
                if had_focus {
                    self.clear_action_error_and_refresh();
                }
                had_focus
            }
            GuiShellAction::BeginAddPublicServer => {
                self.public_server_edit_session = Some(GuiPublicServerEditSessionState {
                    editing_index: None,
                    label_buffer: String::new(),
                    address_buffer: String::new(),
                    is_dirty: false,
                    original_label: None,
                    original_address: None,
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::BeginEditSelectedPublicServer => {
                let Some(index) = self.selected_public_server_index() else {
                    return self.record_action_error("No public server is currently selected.");
                };
                let Some(row) = self.public_servers.servers.get(index) else {
                    return self
                        .record_action_error("No public server exists at the requested index.");
                };
                self.public_server_edit_session = Some(GuiPublicServerEditSessionState {
                    editing_index: Some(index),
                    label_buffer: row.label.clone(),
                    address_buffer: row.address.clone(),
                    is_dirty: false,
                    original_label: Some(row.label.clone()),
                    original_address: Some(row.address.clone()),
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdatePublicServerEditLabel(buffer) => {
                let Some(session) = self.public_server_edit_session.as_mut() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                session.label_buffer = buffer;
                session.is_dirty = true;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdatePublicServerEditAddress(buffer) => {
                let Some(session) = self.public_server_edit_session.as_mut() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                session.address_buffer = buffer;
                session.is_dirty = true;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitPublicServerEdit => {
                let Some(session) = self.public_server_edit_session.clone() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                let label = session.label_buffer.trim();
                let address = session.address_buffer.trim();
                if label.is_empty() || address.is_empty() {
                    return self.record_action_error(
                        "Public-server label and address must both be non-empty.",
                    );
                }
                let (host, _) =
                    parse_host_and_optional_port_from_host_arg_legacy_compatible(address);
                if host.trim().is_empty() {
                    return self.record_action_error("Public-server address is not valid.");
                }

                let mut settings = self.configuration.to_stored_settings();
                let mut servers = settings.public_servers.take().unwrap_or_default();
                if let Some(index) = session.editing_index {
                    if index >= servers.len() {
                        return self.record_action_error(
                            "The public server being edited no longer exists.",
                        );
                    }
                    servers[index] = (label.to_owned(), address.to_owned());
                } else {
                    servers.push((label.to_owned(), address.to_owned()));
                }
                let selected_index = session.editing_index.unwrap_or(servers.len() - 1);
                settings.public_servers = Some(servers);
                self.resync_from_settings(settings);
                self.public_server_edit_session = None;
                if !self.apply_public_server_selection(selected_index) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelPublicServerEdit => {
                if self.public_server_edit_session.is_none() {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                }
                self.public_server_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RemoveSelectedPublicServer => {
                let Some(index) = self.selected_public_server_index() else {
                    return self.record_action_error("No public server is currently selected.");
                };
                let mut settings = self.configuration.to_stored_settings();
                let mut servers = settings.public_servers.take().unwrap_or_default();
                if index >= servers.len() {
                    return self
                        .record_action_error("No public server exists at the requested index.");
                }
                servers.remove(index);
                settings.public_servers = if servers.is_empty() {
                    None
                } else {
                    Some(servers)
                };
                self.resync_from_settings(settings);
                if self.public_servers.servers.is_empty() {
                    self.set_selected_public_server_index(None);
                } else if index >= self.public_servers.servers.len() {
                    self.set_selected_public_server_index(Some(
                        self.public_servers.servers.len() - 1,
                    ));
                } else {
                    self.set_selected_public_server_index(Some(index));
                }
                self.public_server_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::BeginEditSelectedMainWindowUser => {
                let Some(index) = self.selection.selected_main_window_user else {
                    return self.record_action_error("No main-window user is currently selected.");
                };
                let Some(user) = self.main_window.users.get(index) else {
                    return self
                        .record_action_error("No main-window user exists at the requested index.");
                };
                self.main_window_user_edit_session = Some(GuiMainWindowUserEditSessionState {
                    editing_index: index,
                    username_buffer: user.username.clone(),
                    is_dirty: false,
                    original_username: user.username.clone(),
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdateMainWindowUserEdit(buffer) => {
                let Some(session) = self.main_window_user_edit_session.as_mut() else {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                };
                let Some(user) = self.main_window.users.get(session.editing_index) else {
                    self.main_window_user_edit_session = None;
                    return self.record_action_error(
                        "The main-window user being edited no longer exists.",
                    );
                };
                session.is_dirty = buffer != user.username;
                session.username_buffer = buffer;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitMainWindowUserEdit => {
                let Some(session) = self.main_window_user_edit_session.clone() else {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                };
                let Some((previous_username, username)) = self.rename_main_window_user_at_index(
                    session.editing_index,
                    session.username_buffer,
                    "Renamed main-window user names must be non-empty.",
                    "The main-window user being edited no longer exists.",
                ) else {
                    return false;
                };
                self.main_window_user_edit_session = None;
                self.push_transient_notification(
                    GuiTransientNotificationLevel::Success,
                    format!("User renamed: {previous_username} -> {username}."),
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelMainWindowUserEdit => {
                if self.main_window_user_edit_session.is_none() {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                }
                self.main_window_user_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
