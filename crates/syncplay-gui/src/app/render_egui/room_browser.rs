use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use super::{GuiPanelShellOptions, GuiWidgetEguiRenderer};

#[derive(Clone, Copy)]
struct CombinedRoomIdentityNodes<'a> {
    status: Option<&'a GuiWidgetNode>,
    server: Option<&'a GuiWidgetNode>,
    room: Option<&'a GuiWidgetNode>,
    room_control: Option<&'a GuiWidgetNode>,
    playback_state: Option<&'a GuiWidgetNode>,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn render_combined_room_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let status_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:connection-status");
        let server_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:connection-target");
        let room_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:room");
        let room_control_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:room-control");
        let playback_state_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:room-playback-state");
        let header_actions = node
            .children
            .iter()
            .find(|child| child.id == "main-window:room-header:actions");
        let room_actions = node
            .children
            .iter()
            .find(|child| child.id == "main-window:room-actions");
        let participants = node
            .children
            .iter()
            .find(|child| child.id == "main-window:participants");
        let panel_width = Self::panel_available_width(ui);
        let header_content_width = (panel_width - 24.0).max(0.0);
        let compact = header_content_width < 720.0;
        let header_height = if compact {
            if room_actions.is_some() && header_content_width < 360.0 {
                238.0
            } else if room_actions.is_some() {
                138.0
            } else {
                118.0
            }
        } else {
            64.0
        };

        self.render_panel_shell_with_header(
            ui,
            GuiPanelShellOptions::new(panel_width)
                .min_content_height(self.node_min_content_height(node))
                .header_height(header_height)
                .header_content_margin(egui::vec2(12.0, 10.0))
                .body_margin(egui::Margin::same(0))
                .body_horizontal_margin(0.0),
            |renderer, ui, header_content_width| {
                if compact {
                    renderer.render_combined_room_identity_row(
                        ui,
                        CombinedRoomIdentityNodes {
                            status: status_node,
                            server: server_node,
                            room: room_node,
                            room_control: room_control_node,
                            playback_state: playback_state_node,
                        },
                        header_content_width,
                    );
                    if let Some(header_actions) = header_actions {
                        ui.add_space(8.0);
                        renderer.render_combined_room_header_actions(
                            ui,
                            header_actions,
                            state,
                            header_content_width,
                        );
                    }
                } else {
                    ui.horizontal(|ui| {
                        let action_width = header_actions
                            .map_or(0.0_f32, |_| 386.0_f32)
                            .min(header_content_width);
                        let control_width = if room_control_node.is_some() {
                            42.0
                        } else {
                            0.0
                        };
                        let playback_state_width = if playback_state_node.is_some() {
                            42.0
                        } else {
                            0.0
                        };
                        let trailing_icon_width = match (room_control_node, playback_state_node) {
                            (Some(_), Some(_)) => control_width + playback_state_width + 8.0,
                            _ => control_width + playback_state_width,
                        };
                        let identity_width =
                            (header_content_width - action_width - trailing_icon_width - 18.0)
                                .max(160.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(identity_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(identity_width);
                                ui.set_max_width(identity_width);
                                renderer.render_combined_room_identity_row(
                                    ui,
                                    CombinedRoomIdentityNodes {
                                        status: status_node,
                                        server: server_node,
                                        room: room_node,
                                        room_control: None,
                                        playback_state: None,
                                    },
                                    identity_width,
                                );
                            },
                        );
                        if let Some(header_actions) = header_actions {
                            ui.add_space(8.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(action_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(action_width);
                                    ui.set_max_width(action_width);
                                    renderer.render_combined_room_header_actions(
                                        ui,
                                        header_actions,
                                        state,
                                        action_width,
                                    );
                                },
                            );
                        }
                        if playback_state_node.is_some() || room_control_node.is_some() {
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if let Some(playback_state_node) = playback_state_node {
                                    Self::render_playback_state_icon(ui, playback_state_node, 34.0);
                                }
                                if let Some(room_control_node) = room_control_node {
                                    if playback_state_node.is_some() {
                                        ui.add_space(8.0);
                                    }
                                    renderer.render_room_control_icon(ui, room_control_node);
                                }
                            });
                        }
                    });
                }
            },
            |renderer, ui, body_width| {
                if let Some(room_actions) = room_actions {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .fill(Self::palette_for_ui(ui).surface)
                        .show(ui, |ui| {
                            let section_width =
                                Self::width_inside_horizontal_margin(body_width, 24.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(section_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(section_width);
                                    ui.set_max_width(section_width);
                                    renderer.render_room_change_section(ui, room_actions, state);
                                },
                            );
                        });
                    ui.add_space(2.0);
                }

                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        let section_width = Self::width_inside_horizontal_margin(body_width, 24.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(section_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(section_width);
                                ui.set_max_width(section_width);
                                ui.label(
                                    egui::RichText::new("Participants")
                                        .small()
                                        .strong()
                                        .color(Self::palette_for_ui(ui).muted_text),
                                );
                                ui.add_space(6.0);
                                if let Some(participants) = participants {
                                    renderer.render_combined_room_participants(
                                        ui,
                                        participants,
                                        state,
                                    );
                                }
                            },
                        );
                    });
            },
        );
    }

    fn render_combined_room_identity_row(
        &self,
        ui: &mut egui::Ui,
        nodes: CombinedRoomIdentityNodes<'_>,
        row_width: f32,
    ) {
        ui.horizontal(|ui| {
            if let Some(status_node) = nodes.status {
                self.render_connection_status_dot(ui, status_node);
                ui.add_space(8.0);
            }
            let icon_count = usize::from(nodes.room_control.is_some())
                + usize::from(nodes.playback_state.is_some());
            let icon_width = if icon_count == 0 {
                0.0
            } else {
                (icon_count as f32 * 34.0) + ((icon_count.saturating_sub(1)) as f32 * 8.0)
            };
            let text_width = (row_width - icon_width - 34.0).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(text_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(text_width);
                    ui.set_max_width(text_width);
                    let room_label = nodes
                        .room
                        .and_then(|node| node.value.as_deref())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("(no room joined)");
                    let room_response = ui.add(
                        egui::Label::new(
                            egui::RichText::new(room_label)
                                .strong()
                                .color(Self::palette_for_ui(ui).neutral_text),
                        )
                        .truncate(),
                    );
                    if room_label != "(no room joined)" {
                        room_response.on_hover_text(room_label.to_owned());
                    }
                    let server_label = nodes
                        .server
                        .and_then(|node| node.value.as_deref())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("(not configured)");
                    let server_response = ui.add(
                        egui::Label::new(
                            egui::RichText::new(server_label)
                                .small()
                                .color(Self::palette_for_ui(ui).muted_text),
                        )
                        .truncate(),
                    );
                    if server_label != "(not configured)" {
                        server_response.on_hover_text(server_label.to_owned());
                    }
                },
            );
            if nodes.room_control.is_some() || nodes.playback_state.is_some() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(room_control_node) = nodes.room_control {
                        self.render_room_control_icon(ui, room_control_node);
                    }
                    if let Some(playback_state_node) = nodes.playback_state {
                        if nodes.room_control.is_some() {
                            ui.add_space(8.0);
                        }
                        Self::render_playback_state_icon(ui, playback_state_node, 34.0);
                    }
                });
            }
        });
    }

    fn render_connection_status_dot(&self, ui: &mut egui::Ui, node: &GuiWidgetNode) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        let palette = Self::palette_for_ui(ui);
        let color = match node.value.as_deref() {
            Some("connected") => palette.success_text,
            Some("connecting") | Some("disconnecting") => palette.warning_text,
            Some("disconnected") => palette.muted_text,
            _ => palette.neutral_border,
        };
        ui.painter().circle_filled(rect.center(), 6.0, color);
        let _ = Self::attach_node_tooltip(response, node);
    }

    fn render_room_control_icon(&self, ui: &mut egui::Ui, node: &GuiWidgetNode) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
        let palette = Self::palette_for_ui(ui);
        ui.painter().rect(
            rect.shrink(0.5),
            4,
            palette.surface,
            egui::Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
        let status = node.value.as_deref().unwrap_or_default();
        let center = rect.center();
        if status.starts_with("Granted") {
            let stroke = egui::Stroke::new(2.0, palette.primary);
            ui.painter()
                .circle_stroke(egui::pos2(center.x - 5.0, center.y), 4.2, stroke);
            ui.painter().line_segment(
                [
                    egui::pos2(center.x - 1.0, center.y),
                    egui::pos2(center.x + 10.0, center.y),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(center.x + 6.0, center.y),
                    egui::pos2(center.x + 6.0, center.y + 5.0),
                ],
                stroke,
            );
        } else if status.starts_with("Pending") {
            let stroke = egui::Stroke::new(2.0, palette.warning_text);
            ui.painter().circle_stroke(center, 7.0, stroke);
            ui.painter()
                .line_segment([center, egui::pos2(center.x + 5.0, center.y - 5.0)], stroke);
        } else {
            let stroke = egui::Stroke::new(2.0, palette.muted_text);
            ui.painter()
                .circle_stroke(egui::pos2(center.x, center.y - 5.0), 4.0, stroke);
            ui.painter().line_segment(
                [
                    egui::pos2(center.x - 8.0, center.y + 9.0),
                    egui::pos2(center.x + 8.0, center.y + 9.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(center.x - 6.0, center.y + 8.0),
                    egui::pos2(center.x - 2.0, center.y + 2.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(center.x + 6.0, center.y + 8.0),
                    egui::pos2(center.x + 2.0, center.y + 2.0),
                ],
                stroke,
            );
        }
        let _ = Self::attach_node_tooltip(response, node);
    }

    fn render_combined_room_header_actions(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        available_width: f32,
    ) {
        if node.children.is_empty() {
            return;
        }
        ui.allocate_ui_with_layout(
            egui::vec2(available_width.max(0.0), 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(available_width.max(0.0));
                ui.set_max_width(available_width.max(0.0));
                self.render_layout(ui, node, state);
            },
        );
    }

    fn render_room_change_section(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        egui::Frame::new()
            .fill(Self::palette_for_ui(ui).surface_muted)
            .stroke(egui::Stroke::new(1.0, Self::palette_for_ui(ui).border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                let width =
                    Self::width_inside_horizontal_margin(Self::visible_available_width(ui), 20.0);
                ui.set_width(width);
                ui.set_max_width(width);
                for child in &node.children {
                    self.render_node(ui, child, state);
                }
            });
    }

    fn render_combined_room_participants(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let user_nodes: Vec<&GuiWidgetNode> = node
            .children
            .iter()
            .filter(|child| Self::is_room_browser_user_node(child))
            .collect();
        if user_nodes.is_empty() {
            if let Some(empty_node) = node.children.first() {
                ui.label(
                    egui::RichText::new(empty_node.value.as_deref().unwrap_or("No users."))
                        .small()
                        .weak(),
                );
            }
            return;
        }
        for (index, user_node) in user_nodes.iter().enumerate() {
            self.render_combined_room_participant_row(ui, user_node, state);
            if index + 1 < user_nodes.len() {
                ui.add_space(2.0);
            }
        }
    }

    fn render_combined_room_participant_row(
        &mut self,
        ui: &mut egui::Ui,
        user_node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let user_state = Self::find_descendant_by_suffix(user_node, ":state");
        let file_node = Self::find_descendant_by_suffix(user_node, ":file");
        let size_node = Self::find_descendant_by_suffix(user_node, ":size");
        let duration_node = Self::find_descendant_by_suffix(user_node, ":duration");
        let ready_action = user_node
            .children
            .iter()
            .find(|child| child.id == "main-window:control:set-ready");
        let state_value = user_state.and_then(|status| status.value.as_deref());
        let is_ready = state_value.is_some_and(|value| Self::browser_status_flag(value, "ready"));
        let is_controller =
            state_value.is_some_and(|value| Self::browser_status_flag(value, "controller"));
        let (file_text, cues) = file_node
            .and_then(|node| node.value.as_deref())
            .map(Self::browser_file_and_cues)
            .unwrap_or_else(|| ("No file".to_owned(), Vec::new()));
        let metadata = Self::browser_metadata_line(
            size_node.and_then(|node| node.value.as_deref()),
            duration_node.and_then(|node| node.value.as_deref()),
        );
        let palette = Self::palette_for_ui(ui);
        let row_width = Self::visible_available_width(ui);
        let row_content_width = Self::width_inside_horizontal_margin(row_width, 16.0);
        let fill = if user_node.selected {
            palette.surface_muted
        } else {
            egui::Color32::TRANSPARENT
        };

        egui::Frame::new()
            .fill(fill)
            .corner_radius(5)
            .inner_margin(egui::Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.set_width(row_content_width);
                ui.set_max_width(row_content_width);
                ui.horizontal_top(|ui| {
                    Self::render_participant_ready_dot(ui, is_ready);
                    ui.add_space(8.0);
                    let action_width = if ready_action.is_some() {
                        Self::room_ready_button_width(row_width)
                    } else {
                        0.0
                    };
                    let action_gap = if ready_action.is_some() { 8.0 } else { 0.0 };
                    let text_width =
                        (Self::visible_available_width(ui) - action_width - action_gap - 8.0)
                            .max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            ui.set_max_width(text_width);
                            ui.horizontal(|ui| {
                                let name = egui::RichText::new(&user_node.label)
                                    .strong()
                                    .color(Self::palette_for_ui(ui).neutral_text);
                                ui.label(name);
                                if is_controller {
                                    Self::render_inline_controller_icon(ui);
                                }
                            });
                            let file_response = ui.add(
                                egui::Label::new(egui::RichText::new(&file_text).small())
                                    .truncate(),
                            );
                            if !file_text.is_empty() && file_text != "No file" {
                                file_response.on_hover_text(file_text.clone());
                            }
                            if !metadata.is_empty() || !cues.is_empty() {
                                let mut detail_parts = Vec::new();
                                if !metadata.is_empty() {
                                    detail_parts.push(metadata);
                                }
                                if !cues.is_empty() {
                                    detail_parts.push(cues.join(", "));
                                }
                                ui.label(
                                    egui::RichText::new(detail_parts.join(" - "))
                                        .small()
                                        .color(Self::palette_for_ui(ui).muted_text),
                                );
                            }
                        },
                    );
                    if let Some(ready_action) = ready_action {
                        ui.add_space(action_gap);
                        ui.allocate_ui_with_layout(
                            egui::vec2(action_width, 36.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(action_width);
                                ui.set_max_width(action_width);
                                self.render_playback_ready_button(ui, ready_action, state);
                            },
                        );
                    }
                });
            });
    }

    fn room_ready_button_width(row_width: f32) -> f32 {
        if row_width < 420.0 { 128.0 } else { 156.0 }
    }

    fn render_participant_ready_dot(ui: &mut egui::Ui, is_ready: bool) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 28.0), egui::Sense::hover());
        let palette = Self::palette_for_ui(ui);
        let color = if is_ready {
            palette.success_text
        } else {
            palette.warning_text
        };
        ui.painter()
            .circle_filled(egui::pos2(rect.center().x, rect.top() + 12.0), 5.0, color);
        let label = if is_ready { "Ready" } else { "Not ready" };
        let _ = Self::attach_hover_text(response, label);
    }

    fn render_inline_controller_icon(ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::hover());
        let palette = Self::palette_for_ui(ui);
        let stroke = egui::Stroke::new(1.8, palette.primary);
        let center = rect.center();
        ui.painter()
            .circle_stroke(egui::pos2(center.x - 5.0, center.y), 3.8, stroke);
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 1.0, center.y),
                egui::pos2(center.x + 8.0, center.y),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x + 5.0, center.y),
                egui::pos2(center.x + 5.0, center.y + 4.0),
            ],
            stroke,
        );
        let _ = Self::attach_hover_text(response, "Room controller");
    }

    pub(super) fn render_room_browser(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let room_nodes: Vec<&GuiWidgetNode> = node
            .children
            .iter()
            .filter(|child| Self::is_room_browser_room_node(child))
            .collect();
        let user_count = room_nodes
            .iter()
            .map(|room| {
                room.children
                    .iter()
                    .filter(|child| Self::is_room_browser_user_node(child))
                    .count()
            })
            .sum::<usize>();
        let empty_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:browser:empty");
        let hide_empty_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:browser:hide-empty");
        let panel_width = Self::visible_available_width(ui);

        self.render_panel_shell_with_header(
            ui,
            GuiPanelShellOptions::new(panel_width)
                .min_content_height(self.node_min_content_height(node)),
            |renderer, ui, header_width| {
                let right_width = if hide_empty_node.is_some() {
                    178.0
                } else {
                    0.0
                };
                let left_width = (header_width - right_width - 8.0).max(0.0);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, 0.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(
                                    egui::RichText::new(&node.label)
                                        .color(Self::palette_for_ui(ui).neutral_text),
                                );
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(if room_nodes.len() == 1 {
                                        "1 room".to_owned()
                                    } else {
                                        format!("{} rooms", room_nodes.len())
                                    })
                                    .small()
                                    .weak(),
                                );
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(if user_count == 1 {
                                        "1 user".to_owned()
                                    } else {
                                        format!("{user_count} users")
                                    })
                                    .small()
                                    .weak(),
                                );
                                if state.main_window.hide_empty_rooms {
                                    let palette = Self::palette_for_ui(ui);
                                    Self::render_room_browser_chip(
                                        ui,
                                        "Empty Hidden",
                                        palette.info_bg,
                                        palette.info_text,
                                        palette.info_border,
                                    );
                                }
                            });
                        },
                    );
                    if let Some(hide_empty_node) = hide_empty_node {
                        ui.add_space(8.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(right_width, 0.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                renderer.render_leaf(ui, hide_empty_node, state);
                            },
                        );
                    }
                });
            },
            |renderer, ui, body_width| {
                ui.set_width(body_width);
                ui.set_max_width(body_width);
                if room_nodes.is_empty() {
                    let empty_text = empty_node
                        .and_then(|child| child.value.as_deref())
                        .unwrap_or("No visible rooms.");
                    ui.label(egui::RichText::new(empty_text).small().weak());
                } else {
                    let (current_rooms, other_rooms): (Vec<&GuiWidgetNode>, Vec<&GuiWidgetNode>) =
                        room_nodes
                            .iter()
                            .copied()
                            .partition(|room_node| room_node.selected);
                    if !current_rooms.is_empty() {
                        renderer.render_room_browser_room_list(ui, &current_rooms, state);
                    }
                    if !other_rooms.is_empty() {
                        if !current_rooms.is_empty() {
                            ui.add_space(8.0);
                        }
                        ui.strong(
                            egui::RichText::new("Other Rooms")
                                .small()
                                .strong()
                                .color(Self::palette_for_ui(ui).neutral_text),
                        );
                        ui.add_space(4.0);
                        renderer.render_room_browser_room_list(ui, &other_rooms, state);
                    }
                }
            },
        );
    }

    fn render_room_browser_room_list(
        &mut self,
        ui: &mut egui::Ui,
        room_nodes: &[&GuiWidgetNode],
        state: &SyncplayGuiShellAppState,
    ) {
        for (index, room_node) in room_nodes.iter().enumerate() {
            self.render_room_browser_room_card(ui, room_node, state);
            if index + 1 < room_nodes.len() {
                ui.add_space(6.0);
            }
        }
    }

    fn render_room_browser_room_card(
        &mut self,
        ui: &mut egui::Ui,
        room_node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let room_state = Self::find_descendant_by_suffix(room_node, ":state");
        let join_button = Self::find_descendant_by_suffix(room_node, ":join");
        let user_nodes: Vec<&GuiWidgetNode> = room_node
            .children
            .iter()
            .filter(|child| Self::is_room_browser_user_node(child))
            .collect();
        let palette = Self::palette_for_ui(ui);
        let room_fill = if room_node.selected {
            palette.info_bg.gamma_multiply(0.78)
        } else {
            ui.visuals()
                .widgets
                .noninteractive
                .bg_fill
                .gamma_multiply(0.12)
        };
        let room_stroke = if room_node.selected {
            egui::Stroke::new(1.0, palette.info_border)
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 4))
            .fill(room_fill)
            .stroke(room_stroke)
            .corner_radius(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if room_node.selected {
                        ui.label(
                            egui::RichText::new(">")
                                .strong()
                                .color(Self::palette_for_ui(ui).info_text),
                        );
                    }
                    ui.label(
                        egui::RichText::new(format!("{} ({})", room_node.label, user_nodes.len()))
                            .strong()
                            .color(Self::palette_for_ui(ui).neutral_text),
                    );
                    if room_node.selected {
                        let palette = Self::palette_for_ui(ui);
                        Self::render_room_browser_chip(
                            ui,
                            "Current",
                            palette.info_bg,
                            palette.info_text,
                            palette.info_border,
                        );
                    }
                    if room_state
                        .and_then(|status| status.value.as_deref())
                        .is_some_and(|value| Self::browser_status_flag(value, "controlled"))
                    {
                        let palette = Self::palette_for_ui(ui);
                        Self::render_room_browser_chip(
                            ui,
                            "Controlled",
                            palette.controlled_bg,
                            palette.controlled_text,
                            palette.controlled_border,
                        );
                    }
                    if let Some(join_button) = join_button
                        .filter(|button| button.enabled || button.label != "Current Room")
                    {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_room_browser_button(ui, join_button, state)
                        });
                    }
                });

                if !user_nodes.is_empty() {
                    ui.add_space(4.0);
                    self.render_room_browser_user_list(ui, &user_nodes, state);
                } else if let Some(empty_node) = room_node
                    .children
                    .iter()
                    .find(|child| child.id.ends_with(":empty"))
                {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(empty_node.value.as_deref().unwrap_or("(empty room)"))
                            .small()
                            .weak(),
                    );
                }
            });
    }

    fn render_room_browser_user_list(
        &mut self,
        ui: &mut egui::Ui,
        user_nodes: &[&GuiWidgetNode],
        state: &SyncplayGuiShellAppState,
    ) {
        for (index, user_node) in user_nodes.iter().enumerate() {
            self.render_room_browser_user_card(ui, user_node, state);
            if index + 1 < user_nodes.len() {
                ui.add_space(1.0);
            }
        }
    }

    fn render_room_browser_user_card(
        &mut self,
        ui: &mut egui::Ui,
        user_node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let user_state = Self::find_descendant_by_suffix(user_node, ":state");
        let file_node = Self::find_descendant_by_suffix(user_node, ":file");
        let size_node = Self::find_descendant_by_suffix(user_node, ":size");
        let duration_node = Self::find_descendant_by_suffix(user_node, ":duration");
        let action_nodes = [
            Self::find_descendant_by_suffix(user_node, ":open"),
            Self::find_descendant_by_suffix(user_node, ":folder"),
            Self::find_descendant_by_suffix(user_node, ":trust"),
            Self::find_descendant_by_suffix(user_node, ":ready"),
        ];
        let (file_text, cues) = file_node
            .and_then(|node| node.value.as_deref())
            .map(Self::browser_file_and_cues)
            .unwrap_or_else(|| ("(none)".to_owned(), Vec::new()));
        let metadata = Self::browser_metadata_line(
            size_node.and_then(|node| node.value.as_deref()),
            duration_node.and_then(|node| node.value.as_deref()),
        );
        let state_value = user_state.and_then(|status| status.value.as_deref());
        let is_self = state_value.is_some_and(|value| Self::browser_status_flag(value, "self"));
        let is_ready = state_value.is_some_and(|value| Self::browser_status_flag(value, "ready"));
        let is_controller =
            state_value.is_some_and(|value| Self::browser_status_flag(value, "controller"));
        let palette = Self::palette_for_ui(ui);
        let card_fill = if user_node.selected {
            palette.info_bg.gamma_multiply(0.62)
        } else {
            egui::Color32::TRANSPARENT
        };
        let card_stroke = egui::Stroke::NONE;

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 3))
            .fill(card_fill)
            .stroke(card_stroke)
            .corner_radius(0)
            .show(ui, |ui| {
                let visible_actions: Vec<&GuiWidgetNode> = action_nodes
                    .into_iter()
                    .flatten()
                    .filter(|node| {
                        node.enabled
                            || matches!(node.id.as_str(), id if id.ends_with(":ready") || id.ends_with(":open"))
                    })
                    .collect();

                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    Self::render_room_browser_ready_icon(ui, is_ready);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(&user_node.label)
                                    .strong()
                                    .color(Self::palette_for_ui(ui).neutral_text),
                            );
                            if is_self {
                                let palette = Self::palette_for_ui(ui);
                                Self::render_room_browser_chip(
                                    ui,
                                    "You",
                                    palette.info_bg,
                                    palette.info_text,
                                    palette.info_border,
                                );
                            }
                            if is_controller {
                                let palette = Self::palette_for_ui(ui);
                                Self::render_room_browser_chip(
                                    ui,
                                    "Controller",
                                    palette.controlled_bg,
                                    palette.controlled_text,
                                    palette.controlled_border,
                                );
                            }
                            for cue in &cues {
                                let palette = Self::palette_for_ui(ui);
                                Self::render_room_browser_chip(
                                    ui,
                                    cue,
                                    palette.warning_bg,
                                    palette.warning_text,
                                    palette.warning_border,
                                );
                            }
                        });

                        let file_response = ui.add(
                            egui::Label::new(egui::RichText::new(&file_text).small()).truncate(),
                        );
                        if !file_text.is_empty() && file_text != "(none)" {
                            file_response.on_hover_text(file_text.clone());
                        }
                        if !metadata.is_empty() {
                            ui.label(egui::RichText::new(metadata).small().weak());
                        }
                    });
                });
                if !visible_actions.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(32.0);
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 6.0;
                        ui.spacing_mut().item_spacing = spacing;
                        for action in visible_actions {
                            self.render_room_browser_button(ui, action, state);
                        }
                    });
                }
            });
    }

    fn render_room_browser_ready_icon(ui: &mut egui::Ui, is_ready: bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        let palette = Self::palette_for_ui(ui);
        let color = if is_ready {
            palette.success_text
        } else {
            palette.warning_text
        };
        let stroke = egui::Stroke::new(2.1, color);
        if is_ready {
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 3.0, rect.center().y),
                    egui::pos2(rect.left() + 7.0, rect.bottom() - 4.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 7.0, rect.bottom() - 4.0),
                    egui::pos2(rect.right() - 3.0, rect.top() + 4.0),
                ],
                stroke,
            );
        } else {
            let icon_rect = rect.shrink2(egui::vec2(4.0, 4.0));
            ui.painter()
                .line_segment([icon_rect.left_top(), icon_rect.right_bottom()], stroke);
            ui.painter()
                .line_segment([icon_rect.right_top(), icon_rect.left_bottom()], stroke);
        }
    }

    fn render_room_browser_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let label = Self::room_browser_button_label(node);
        let response = ui.add_enabled(
            node.enabled,
            egui::Button::new(egui::RichText::new(&label).small())
                .small()
                .min_size(egui::vec2(0.0, 22.0))
                .corner_radius(6),
        );
        let response = if label != node.label {
            Self::attach_hover_text(response, node.label.clone())
        } else {
            response
        };
        if response.clicked() {
            self.handle_button_node_click(state, node);
        }
    }

    fn render_room_browser_chip(
        ui: &mut egui::Ui,
        label: impl Into<String>,
        fill: egui::Color32,
        text_color: egui::Color32,
        stroke_color: egui::Color32,
    ) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 2))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke_color.gamma_multiply(0.55)))
            .corner_radius(6)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(label.into()).small().color(text_color));
            });
    }

    fn is_room_browser_room_node(node: &GuiWidgetNode) -> bool {
        node.kind == GuiWidgetKind::Panel && node.id.starts_with("main-window:room-group:")
    }

    fn is_room_browser_user_node(node: &GuiWidgetNode) -> bool {
        node.kind == GuiWidgetKind::Panel && node.id.starts_with("main-window:user:")
    }

    fn find_descendant_by_suffix<'a>(
        node: &'a GuiWidgetNode,
        suffix: &str,
    ) -> Option<&'a GuiWidgetNode> {
        if node.id.ends_with(suffix) {
            return Some(node);
        }
        node.children
            .iter()
            .find_map(|child| Self::find_descendant_by_suffix(child, suffix))
    }

    fn browser_status_flag(value: &str, key: &str) -> bool {
        value.split(',').any(|entry| {
            let mut parts = entry.trim().splitn(2, '=');
            matches!(
                (parts.next(), parts.next()),
                (Some(flag), Some("yes" | "true")) if flag == key
            )
        })
    }

    fn browser_file_and_cues(value: &str) -> (String, Vec<String>) {
        let (file_text, cue_text) = value
            .ends_with(']')
            .then(|| value.rsplit_once(" ["))
            .flatten()
            .map_or((value, None), |(file_text, cue_text)| {
                (file_text, Some(cue_text))
            });
        let cues = cue_text
            .map(|cue_text| {
                cue_text
                    .trim_end_matches(']')
                    .split(',')
                    .filter_map(|cue| Self::browser_cue_label(cue.trim(), file_text))
                    .collect()
            })
            .unwrap_or_default();
        (file_text.to_owned(), cues)
    }

    fn browser_cue_label(cue: &str, file_text: &str) -> Option<String> {
        match cue {
            "no-file" if file_text.eq_ignore_ascii_case("No file") => None,
            "no-file" => Some("No File".to_owned()),
            "name-diff" => Some("Name Diff".to_owned()),
            "size-diff" => Some("Size Diff".to_owned()),
            "duration-diff" => Some("Duration Diff".to_owned()),
            "untrusted-url" => Some("Untrusted".to_owned()),
            _ if cue.is_empty() => None,
            _ => Some(cue.replace('-', " ")),
        }
    }

    fn browser_metadata_line(size: Option<&str>, duration: Option<&str>) -> String {
        [size, duration]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "(none)")
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .join(" - ")
    }

    fn room_browser_button_label(node: &GuiWidgetNode) -> String {
        if node.id.ends_with(":join") {
            if node.label == "Current Room" {
                "Current".to_owned()
            } else {
                "Join".to_owned()
            }
        } else if node.id.ends_with(":open") {
            if node.label == "Open Stream" {
                "Open Stream".to_owned()
            } else {
                "Open".to_owned()
            }
        } else if node.id.ends_with(":folder") {
            "Folder".to_owned()
        } else if node.id.ends_with(":ready") {
            if node.label.contains(" Not Ready") {
                "Not Ready".to_owned()
            } else {
                "Ready".to_owned()
            }
        } else if node.id.ends_with(":trust") {
            node.label
                .strip_prefix("Trust ")
                .filter(|suffix| !suffix.is_empty() && *suffix != "Domain")
                .map_or_else(|| "Trust".to_owned(), |suffix| format!("Trust {suffix}"))
        } else {
            Self::display_text(node)
        }
    }
}
