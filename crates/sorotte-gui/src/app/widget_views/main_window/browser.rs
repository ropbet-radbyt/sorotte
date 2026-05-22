use super::*;

impl SorotteGuiShellAppState {
    pub(crate) fn main_window_browser_widget_node(&self) -> GuiWidgetNode {
        let can_join_room =
            self.pending_operation.is_none() && self.commands.can_disconnect_session;
        let can_open_media =
            self.pending_operation.is_none() && self.media_open_runtime_available();
        let can_mutate_browser_settings = self.pending_operation.is_none();
        let mut room_children = Vec::new();

        for (room_index, room) in self.main_window.rooms.iter().enumerate() {
            if self.main_window.hide_empty_rooms && !room.has_named_users {
                continue;
            }

            let current_room = room.room_name == self.main_window.room_name;
            let mut room_group_children = vec![
                GuiWidgetNode::layout(
                    format!("main-window:room-group:{room_index}:summary"),
                    format!("{} Summary", room.room_name),
                    GuiLayoutMode::KeyValueGrid {
                        min_pair_width: 220.0,
                    },
                    vec![GuiWidgetNode::leaf(
                        format!("main-window:room-group:{room_index}:state"),
                        "State",
                        GuiWidgetKind::Status,
                        Some(format!(
                            "current={}, controlled={}, named_users={}",
                            bool_label(current_room),
                            bool_label(room.is_controlled),
                            bool_label(room.has_named_users),
                        )),
                        true,
                        false,
                    )],
                ),
                GuiWidgetNode::layout(
                    format!("main-window:room-group:{room_index}:actions"),
                    format!("{} Actions", room.room_name),
                    GuiLayoutMode::ButtonWrap {
                        min_button_width: 140.0,
                    },
                    vec![GuiWidgetNode::leaf(
                        format!("main-window:room-group:{room_index}:join"),
                        if current_room {
                            "Current Room"
                        } else {
                            "Join Room"
                        },
                        GuiWidgetKind::Button,
                        None,
                        can_join_room && !current_room,
                        false,
                    )],
                ),
            ];

            let mut has_visible_users = false;
            for (user_index, user) in self.main_window.users.iter().enumerate() {
                if user.room_name != room.room_name {
                    continue;
                }
                has_visible_users = true;

                let mut cue_parts = Vec::new();
                if !user.has_file {
                    cue_parts.push("no-file".to_owned());
                }
                if user.filename_differs {
                    cue_parts.push("name-diff".to_owned());
                }
                if user.filesize_differs {
                    cue_parts.push("size-diff".to_owned());
                }
                if user.fileduration_differs {
                    cue_parts.push("duration-diff".to_owned());
                }
                if user.file_is_url && !user.file_is_trusted {
                    cue_parts.push("untrusted-url".to_owned());
                }
                let cue_suffix = if cue_parts.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", cue_parts.join(", "))
                };
                let trusted_domain = user
                    .file_name
                    .as_deref()
                    .filter(|file_name| browser_is_url(file_name) && !user.file_is_trusted)
                    .and_then(browser_domain_from_url);
                let can_change_ready = self.can_request_main_window_user_ready_change(user);

                let mut user_panel = GuiWidgetNode::branch(
                    format!("main-window:user:{user_index}"),
                    &user.username,
                    GuiWidgetKind::Panel,
                    vec![
                        GuiWidgetNode::layout(
                            format!("main-window:user:{user_index}:summary"),
                            format!("{} Summary", user.username),
                            GuiLayoutMode::KeyValueGrid {
                                min_pair_width: 220.0,
                            },
                            vec![
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:state"),
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
                                    format!("main-window:user:{user_index}:size"),
                                    "Size",
                                    GuiWidgetKind::Status,
                                    Some(if user.file_size_label.is_empty() {
                                        "(none)".to_owned()
                                    } else {
                                        user.file_size_label.clone()
                                    }),
                                    true,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:duration"),
                                    "Duration",
                                    GuiWidgetKind::Status,
                                    Some(if user.file_duration_label.is_empty() {
                                        "(none)".to_owned()
                                    } else {
                                        user.file_duration_label.clone()
                                    }),
                                    true,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:file"),
                                    "File",
                                    GuiWidgetKind::Status,
                                    Some(format!("{}{}", user.file_name_label, cue_suffix)),
                                    true,
                                    false,
                                ),
                            ],
                        ),
                        GuiWidgetNode::layout(
                            format!("main-window:user:{user_index}:actions"),
                            format!("{} Actions", user.username),
                            GuiLayoutMode::ButtonWrap {
                                min_button_width: 140.0,
                            },
                            vec![
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:open"),
                                    if user.file_is_url {
                                        "Open Stream"
                                    } else {
                                        "Open User File"
                                    },
                                    GuiWidgetKind::Button,
                                    None,
                                    can_open_media && user.has_file && !user.is_self,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:folder"),
                                    "Open Containing Folder",
                                    GuiWidgetKind::Button,
                                    None,
                                    can_mutate_browser_settings
                                        && user.has_file
                                        && !user.file_is_url,
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:trust"),
                                    trusted_domain
                                        .as_deref()
                                        .map(|domain| format!("Trust {domain}"))
                                        .unwrap_or_else(|| "Trust Domain".to_owned()),
                                    GuiWidgetKind::Button,
                                    None,
                                    can_mutate_browser_settings && trusted_domain.is_some(),
                                    false,
                                ),
                                GuiWidgetNode::leaf(
                                    format!("main-window:user:{user_index}:ready"),
                                    if user.is_ready {
                                        format!("Set {} Not Ready", user.username)
                                    } else {
                                        format!("Set {} Ready", user.username)
                                    },
                                    GuiWidgetKind::Button,
                                    None,
                                    can_change_ready,
                                    false,
                                ),
                            ],
                        ),
                    ],
                );
                user_panel.selected = user.is_selected;
                room_group_children.push(user_panel);
            }

            if !has_visible_users {
                room_group_children.push(GuiWidgetNode::leaf(
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
                room_group_children,
            );
            room_panel.selected = current_room;
            room_children.push(room_panel);
        }

        if room_children.is_empty() {
            room_children.push(GuiWidgetNode::leaf(
                "main-window:browser:empty",
                "Room Browser",
                GuiWidgetKind::Status,
                Some("No visible rooms.".to_owned()),
                true,
                false,
            ));
        }

        room_children.insert(
            0,
            GuiWidgetNode::leaf(
                "main-window:browser:hide-empty",
                "Hide Empty Rooms",
                GuiWidgetKind::Checkbox,
                Some(bool_label(self.main_window.hide_empty_rooms).to_owned()),
                can_mutate_browser_settings,
                false,
            ),
        );

        GuiWidgetNode::branch(
            "main-window:browser",
            "Room Browser",
            GuiWidgetKind::Panel,
            room_children,
        )
    }
}
