use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn configuration_widget_tree(&self) -> GuiWidgetNode {
        let busy = self.pending_operation.is_some();
        let mut children =
            self.configuration
                .sections
                .iter()
                .map(|section| {
                    let controls = section
                        .controls
                        .iter()
                        .map(|control| {
                            let active_edit_session =
                                self.text_edit_session.as_ref().and_then(|session| {
                                    ((session.section == section.title)
                                        && (session.label == control.label))
                                        .then_some(session)
                                });
                            let focused = self.focused_configuration_control.as_ref().is_some_and(
                                |focused| {
                                    focused.section == section.title
                                        && focused.label == control.label
                                },
                            );
                            let value = active_edit_session
                                .map(|session| session.buffer.clone())
                                .unwrap_or_else(|| control.value.clone());
                            GuiWidgetNode::leaf(
                                format!("config:{}:{}", section.title, control.label),
                                control.label,
                                control.kind.widget_kind(),
                                Some(value),
                                control.kind.is_editable() && !busy,
                                focused,
                            )
                        })
                        .collect();

                    GuiWidgetNode::branch(
                        format!("config-section:{}", section.title),
                        section.title,
                        GuiWidgetKind::Panel,
                        controls,
                    )
                })
                .collect::<Vec<_>>();

        children.push(GuiWidgetNode::branch(
            "config-commands",
            "Commands",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "config-command:edit-room-history",
                    "Edit Room History",
                    GuiWidgetKind::Button,
                    None,
                    self.pending_operation.is_none(),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:connect",
                    self.saved_session_connect_button_label(),
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_connect_saved_server,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:disconnect",
                    "Disconnect",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_disconnect_session,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:save",
                    "Save",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_save_configuration,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:reset",
                    "Reset",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_reset_configuration,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:reload",
                    "Reload",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_reload_configuration,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "config-command:clear-gui-data",
                    "Clear GUI Data",
                    GuiWidgetKind::Button,
                    None,
                    self.pending_operation.is_none(),
                    false,
                ),
            ],
        ));

        if let Some(session) = &self.room_history_edit_session {
            children.push(GuiWidgetNode::branch(
                "room-history:edit-session",
                "Room History Edit",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "room-history:edit:entries",
                        "Room History Entries",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        self.pending_operation.is_none(),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "room-history:edit:commit",
                        "Save Room History",
                        GuiWidgetKind::Button,
                        None,
                        session.is_dirty,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "room-history:edit:cancel",
                        "Cancel Room History Edit",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                ],
            ));
        }

        GuiWidgetNode::branch(
            "configuration-root",
            "Configuration",
            GuiWidgetKind::Panel,
            children,
        )
    }
}
