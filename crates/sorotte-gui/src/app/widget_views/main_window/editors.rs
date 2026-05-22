use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn main_window_editor_panels(&self) -> Vec<GuiWidgetNode> {
        let media_url_edit_panel = self.media_url_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:media-url-edit",
                "Open URL",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:media-url-edit:form",
                        "Open URL Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:media-url-edit:text",
                            "URL",
                            GuiWidgetKind::TextInput,
                            Some(session.buffer.clone()),
                            self.pending_operation.is_none(),
                            false,
                        )],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:media-url-edit:actions",
                        "Open URL Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:media-url-edit:commit",
                                "Open URL",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:media-url-edit:cancel",
                                "Cancel Open URL",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                        ],
                    ),
                ],
            )
        });

        let controlled_room_create_panel =
            self.controlled_room_create_session.as_ref().map(|session| {
                let can_create_controlled_room = normalized_editable_text(
                    &controlled_room_base_name_legacy_compatible(&session.room_buffer),
                )
                .is_some();
                GuiWidgetNode::branch(
                    "main-window:controlled-room-create",
                    "Create Controlled Room",
                    GuiWidgetKind::Panel,
                    vec![
                        GuiWidgetNode::layout(
                            "main-window:controlled-room-create:form",
                            "Controlled Room Form",
                            GuiLayoutMode::FormGrid {
                                label_width: 160.0,
                                min_field_width: 220.0,
                            },
                            vec![GuiWidgetNode::leaf(
                                "main-window:controlled-room-create:room",
                                "Room Name",
                                GuiWidgetKind::TextInput,
                                Some(session.room_buffer.clone()),
                                self.pending_operation.is_none(),
                                false,
                            )],
                        ),
                        GuiWidgetNode::layout(
                            "main-window:controlled-room-create:actions",
                            "Controlled Room Actions",
                            GuiLayoutMode::ButtonWrap {
                                min_button_width: 140.0,
                            },
                            vec![
                                GuiWidgetNode::leaf(
                                    "main-window:controlled-room-create:commit",
                                    "Create Controlled Room",
                                    GuiWidgetKind::Button,
                                    None,
                                    can_create_controlled_room,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    "main-window:controlled-room-create:cancel",
                                    "Cancel Controlled Room Creation",
                                    GuiWidgetKind::Button,
                                    None,
                                    true,
                                    false,
                                ),
                            ],
                        ),
                    ],
                )
            });

        let controller_auth_panel = self.controller_auth_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:controller-auth",
                "Identify As Controller",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::layout(
                        "main-window:controller-auth:form",
                        "Controller Auth Form",
                        GuiLayoutMode::FormGrid {
                            label_width: 160.0,
                            min_field_width: 220.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:room",
                                "Room",
                                GuiWidgetKind::Status,
                                Some(session.room_name.clone()),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:password",
                                "Password",
                                GuiWidgetKind::PasswordInput,
                                Some(session.password_buffer.clone()),
                                self.pending_operation.is_none(),
                                false,
                            ),
                        ],
                    ),
                    GuiWidgetNode::layout(
                        "main-window:controller-auth:actions",
                        "Controller Auth Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:commit",
                                "Identify As Controller",
                                GuiWidgetKind::Button,
                                None,
                                session.is_dirty,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "main-window:controller-auth:cancel",
                                "Cancel Controller Auth",
                                GuiWidgetKind::Button,
                                None,
                                true,
                                false,
                            ),
                        ],
                    ),
                ],
            )
        });

        [
            media_url_edit_panel,
            controlled_room_create_panel,
            controller_auth_panel,
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}
