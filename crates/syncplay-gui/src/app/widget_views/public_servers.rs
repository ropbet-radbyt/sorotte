use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn public_server_widget_tree(&self) -> GuiWidgetNode {
        let has_selected_server = self.selected_public_server_index().is_some();
        let can_run_server_commands =
            self.pending_operation.is_none() && self.public_server_edit_session.is_none();
        let server_list = GuiWidgetNode::branch(
            "public-servers:list",
            "Servers",
            GuiWidgetKind::List,
            self.public_servers
                .servers
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("public-servers:row:{index}"),
                        &row.label,
                        GuiWidgetKind::ListItem,
                        Some(row.address.clone()),
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        );

        let commands = GuiWidgetNode::layout(
            "public-servers:commands",
            "Commands",
            GuiLayoutMode::Stack,
            vec![GuiWidgetNode::layout(
                "public-servers:commands:buttons",
                "Server Commands",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "public-servers:command:connect",
                        "Connect",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_connect_public_server && can_run_server_commands,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:command:refresh",
                        "Refresh",
                        GuiWidgetKind::Button,
                        None,
                        self.commands.can_refresh_public_servers && can_run_server_commands,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:command:add-custom",
                        "Add Custom Server",
                        GuiWidgetKind::Button,
                        None,
                        self.public_servers.can_add_custom_server && can_run_server_commands,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:command:edit",
                        "Edit Selected",
                        GuiWidgetKind::Button,
                        None,
                        has_selected_server && can_run_server_commands,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "public-servers:command:remove",
                        "Remove Selected",
                        GuiWidgetKind::Button,
                        None,
                        has_selected_server && can_run_server_commands,
                        false,
                    ),
                ],
            )],
        );

        let mut children = vec![server_list.with_min_content_height(80.0), commands];
        if let Some(session) = &self.public_server_edit_session {
            children.push(GuiWidgetNode::branch(
                "public-servers:edit-session",
                "Edit Session",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "public-servers:edit:form",
                        "Edit Server Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "public-servers:edit:label",
                                "Label",
                                GuiWidgetKind::TextInput,
                                Some(session.label_buffer.clone()),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "public-servers:edit:address",
                                "Address",
                                GuiWidgetKind::TextInput,
                                Some(session.address_buffer.clone()),
                                true,
                                false,
                            ),
                        ],
                    ),
                    GuiWidgetNode::layout(
                        "public-servers:edit:actions",
                        "Edit Session Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "public-servers:edit:commit",
                                "Save Changes",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "public-servers:edit:cancel",
                                "Cancel Edit",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                        ],
                    ),
                ],
            ));
        }

        GuiWidgetNode::branch(
            "public-servers-root",
            "Saved / Public Servers",
            GuiWidgetKind::Panel,
            children,
        )
    }
}
