use super::*;

mod chat;
mod editors;
mod playlist;
mod summary;

impl SorotteGuiShellAppState {
    pub(crate) fn main_window_widget_tree(&self) -> GuiWidgetNode {
        let (player_setup_panel, summary_column) = self.main_window_summary_projection();

        let playlist_column = self.main_window_playlist_column();
        let chat_panel = self.main_window_chat_panel();

        let top_region = GuiWidgetNode::layout(
            "main-window:top-region",
            "Room Dashboard",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 240.0,
                max_columns: 3,
            },
            vec![
                summary_column.clone(),
                playlist_column.clone(),
                chat_panel.clone(),
            ],
        );

        let mut overview_children = vec![top_region, self.main_window_room_filter_widget_node()];
        let mut overview_editor_panels = self.main_window_editor_panels();
        if !overview_editor_panels.is_empty() {
            if overview_editor_panels.len() == 1
                && let Some(editor_panel) = overview_editor_panels.first_mut()
            {
                *editor_panel = editor_panel.clone().with_span(2);
            }
            overview_children.push(GuiWidgetNode::layout(
                "main-window:editors",
                "Room Editors",
                GuiLayoutMode::ResponsiveColumns {
                    min_column_width: 420.0,
                    max_columns: 2,
                },
                overview_editor_panels,
            ));
        }

        let overview_content = GuiWidgetNode::layout(
            "main-window:content",
            "Room Content",
            GuiLayoutMode::Stack,
            overview_children,
        );

        GuiWidgetNode::layout(
            "main-window-root",
            "Room",
            GuiLayoutMode::Stack,
            player_setup_panel
                .into_iter()
                .chain([overview_content])
                .collect(),
        )
    }

    fn main_window_room_filter_widget_node(&self) -> GuiWidgetNode {
        let can_change_filter = self.pending_operation.is_none();
        let mut children = vec![GuiWidgetNode::leaf(
            "main-window:browser:hide-empty",
            "Hide Empty Rooms",
            GuiWidgetKind::Checkbox,
            Some(bool_label(self.main_window.hide_empty_rooms).to_owned()),
            can_change_filter,
            false,
        )];

        children.extend(
            self.main_window
                .rooms
                .iter()
                .enumerate()
                .filter(|(_, room)| !self.main_window.hide_empty_rooms || room.has_named_users)
                .map(|(room_index, room)| {
                    let mut room_children = vec![GuiWidgetNode::leaf(
                        format!("main-window:room-group:{room_index}:state"),
                        "State",
                        GuiWidgetKind::Status,
                        Some(format!(
                            "controlled={}, named_users={}",
                            bool_label(room.is_controlled),
                            bool_label(room.has_named_users),
                        )),
                        true,
                        false,
                    )];
                    room_children.extend(
                        self.main_window
                            .users
                            .iter()
                            .enumerate()
                            .filter(|(_, user)| user.room_name == room.room_name)
                            .map(|(user_index, user)| {
                                let mut user_panel = GuiWidgetNode::branch(
                                    format!("main-window:user:browser:{user_index}"),
                                    &user.username,
                                    GuiWidgetKind::Panel,
                                    vec![
                                        GuiWidgetNode::leaf(
                                            format!("main-window:user:browser:{user_index}:state"),
                                            "State",
                                            GuiWidgetKind::Status,
                                            Some(format!(
                                                "self={}, ready={}, controller={}",
                                                bool_label(user.is_self),
                                                bool_label(user.is_ready),
                                                bool_label(user.is_controller),
                                            )),
                                            true,
                                            user.is_selected,
                                        ),
                                        GuiWidgetNode::leaf(
                                            format!("main-window:user:browser:{user_index}:file"),
                                            "File",
                                            GuiWidgetKind::Status,
                                            Some(user.file_name_label.clone()),
                                            true,
                                            false,
                                        ),
                                    ],
                                );
                                user_panel.selected = user.is_selected;
                                user_panel
                            }),
                    );
                    if room_children.len() == 1 {
                        room_children.push(GuiWidgetNode::leaf(
                            format!("main-window:room-group:{room_index}:empty"),
                            "Users",
                            GuiWidgetKind::Status,
                            Some("(empty room)".to_owned()),
                            true,
                            false,
                        ));
                    }
                    let mut room_panel = GuiWidgetNode::branch(
                        format!("main-window:room-group:{room_index}"),
                        &room.room_name,
                        GuiWidgetKind::Panel,
                        room_children,
                    );
                    room_panel.selected = room.room_name == self.main_window.room_name;
                    room_panel
                }),
        );

        if children.len() == 1 {
            children.push(GuiWidgetNode::leaf(
                "main-window:browser:empty",
                "Rooms",
                GuiWidgetKind::Status,
                Some("No visible rooms.".to_owned()),
                true,
                false,
            ));
        }

        GuiWidgetNode::branch(
            "main-window:browser",
            "Room Browser",
            GuiWidgetKind::Panel,
            children,
        )
    }
}
