use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use super::GuiWidgetEguiRenderer;

impl GuiWidgetEguiRenderer {
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
        let frame = egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(0))
            .fill(ui.visuals().extreme_bg_color.gamma_multiply(0.18));

        frame.show(ui, |ui| {
            if let Some(min_content_height) = node.min_content_height {
                ui.set_min_height(min_content_height);
            }

            let header_width = ui.available_width().max(0.0);
            egui::Frame::new()
                .fill(Self::panel_header_fill(ui))
                .stroke(Self::panel_header_stroke(ui))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(header_width);
                    ui.horizontal(|ui| {
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
                        if let Some(hide_empty_node) = hide_empty_node {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    self.render_leaf(ui, hide_empty_node, state);
                                },
                            );
                        }
                    });
                });

            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    if room_nodes.is_empty() {
                        let empty_text = empty_node
                            .and_then(|child| child.value.as_deref())
                            .unwrap_or("No visible rooms.");
                        ui.label(egui::RichText::new(empty_text).small().weak());
                    } else {
                        let (current_rooms, other_rooms): (
                            Vec<&GuiWidgetNode>,
                            Vec<&GuiWidgetNode>,
                        ) = room_nodes
                            .iter()
                            .copied()
                            .partition(|room_node| room_node.selected);
                        if !current_rooms.is_empty() {
                            self.render_room_browser_room_list(ui, &current_rooms, state);
                        }
                        if !other_rooms.is_empty() {
                            if !current_rooms.is_empty() {
                                ui.add_space(8.0);
                            }
                            ui.label(egui::RichText::new("Other Rooms").small().strong());
                            ui.add_space(4.0);
                            self.render_room_browser_room_list(ui, &other_rooms, state);
                        }
                    }
                });
        });
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
                    let prefix = if room_node.selected { "✓" } else { "−" };
                    ui.label(egui::RichText::new(prefix).strong());
                    ui.strong(format!("{} ({})", room_node.label, user_nodes.len()));
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
                            ui.label(egui::RichText::new(&user_node.label).strong());
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
                    if !visible_actions.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut spacing = ui.spacing().item_spacing;
                            spacing.x = 6.0;
                            ui.spacing_mut().item_spacing = spacing;
                            for action in visible_actions.into_iter().rev() {
                                self.render_room_browser_button(ui, action, state);
                            }
                        });
                    }
                });
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
