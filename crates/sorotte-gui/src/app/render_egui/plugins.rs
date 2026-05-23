use eframe::egui;

use super::super::shell_state::SorotteGuiShellAppState;
use super::super::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use super::{GuiPanelShellOptions, GuiWidgetEguiRenderer};

impl GuiWidgetEguiRenderer {
    const PLUGINS_GAP: f32 = 12.0;
    const PLUGINS_STACK_BREAKPOINT: f32 = 760.0;

    pub(super) fn render_plugins_surface(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let Some(plugin_list) = node.find("plugins:list") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(details) = node.find("plugins:details") else {
            self.render_layout(ui, node, state);
            return;
        };

        let available_width = Self::visible_available_width(ui);
        let content_width = available_width.clamp(0.0, 1320.0);
        let side_space = ((available_width - content_width) * 0.5).max(0.0);

        ui.horizontal_top(|ui| {
            if side_space > 0.0 {
                ui.add_space(side_space);
            }
            Self::allocate_plugin_width(ui, content_width, |ui| {
                if let Some((rail_width, detail_width)) =
                    Self::plugins_surface_split_for_width(content_width)
                {
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = Self::PLUGINS_GAP;
                        ui.spacing_mut().item_spacing = spacing;
                        Self::allocate_plugin_width(ui, rail_width, |ui| {
                            self.render_plugins_list_panel(ui, plugin_list, state, rail_width);
                        });
                        Self::allocate_plugin_width(ui, detail_width, |ui| {
                            self.render_plugins_detail_stack(ui, details, state, detail_width);
                        });
                    });
                } else {
                    self.render_plugins_list_panel(ui, plugin_list, state, content_width);
                    ui.add_space(Self::PLUGINS_GAP);
                    self.render_plugins_detail_stack(ui, details, state, content_width);
                }
            });
        });
    }

    pub(super) fn plugins_surface_split_for_width(width: f32) -> Option<(f32, f32)> {
        let width = width.max(0.0);
        if width < Self::PLUGINS_STACK_BREAKPOINT {
            return None;
        }
        let rail_width = (width * 0.22).clamp(220.0, 280.0);
        let detail_width = (width - rail_width - Self::PLUGINS_GAP).max(0.0);
        if detail_width < 420.0 {
            None
        } else {
            Some((rail_width, detail_width))
        }
    }

    fn allocate_plugin_width(
        ui: &mut egui::Ui,
        width: f32,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let width = width.max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width);
                ui.set_max_width(width);
                add_contents(ui);
            },
        );
    }

    fn render_plugins_list_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        panel_width: f32,
    ) {
        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width).body_margin(egui::Margin::symmetric(10, 10)),
            |renderer, ui, body_width| {
                for child in &node.children {
                    renderer.render_plugin_list_item(ui, child, state, body_width);
                }
            },
        );
    }

    fn render_plugins_detail_stack(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        panel_width: f32,
    ) {
        for (index, child) in node.children.iter().enumerate() {
            if child.id == "plugins:stream-support" {
                self.render_stream_support_plugin_panel(ui, child, state, panel_width);
            } else if child.id == "plugins:media-matching" {
                self.render_media_matching_plugin_panel(ui, child, state, panel_width);
            } else if child.id == "plugins:plex" {
                self.render_plex_plugin_panel(ui, child, state, panel_width);
            } else {
                self.render_layout(ui, child, state);
            }
            if index + 1 < node.children.len() {
                ui.add_space(Self::PLUGINS_GAP);
            }
        }
    }

    fn render_plugin_list_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        _state: &SorotteGuiShellAppState,
        row_width: f32,
    ) {
        let row_height = 46.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(row_width, row_height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &node.label)
        });
        if response.clicked()
            && let Some(action) = Self::action_for_list_item_node(node)
        {
            self.actions.push(action);
        }

        let palette = Self::palette_for_ui(ui);
        let fill = if node.selected {
            palette.info_bg
        } else if response.hovered() {
            palette.surface_muted
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = if node.selected {
            egui::Stroke::new(1.0, palette.info_border)
        } else if response.hovered() {
            egui::Stroke::new(1.0, palette.border)
        } else {
            egui::Stroke::NONE
        };
        let row_rect = rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter()
            .rect(row_rect, 5, fill, stroke, egui::StrokeKind::Inside);

        let content_rect = row_rect.shrink2(egui::vec2(10.0, 7.0));
        let label_color = if node.selected {
            palette.info_text
        } else {
            palette.neutral_text
        };
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let value = node.value.as_deref().unwrap_or("");
        let chip_width = if value.is_empty() { 0.0 } else { 70.0 };
        let label_width = (content_rect.width() - chip_width - 8.0).max(0.0);
        let (display_label, truncated) = Self::truncate_single_line_text_for_width(
            ui,
            &node.label,
            font_id.clone(),
            label_color,
            label_width,
        );
        let galley = ui
            .painter()
            .layout_no_wrap(display_label, font_id, label_color);
        ui.painter().with_clip_rect(content_rect).galley(
            egui::pos2(
                content_rect.left(),
                content_rect.center().y - (galley.size().y * 0.5),
            ),
            galley,
            label_color,
        );
        if truncated {
            response.on_hover_text(node.label.clone());
        }

        if !value.is_empty() {
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    content_rect.right() - chip_width,
                    content_rect.center().y - 11.0,
                ),
                egui::vec2(chip_width, 22.0),
            );
            Self::paint_stream_support_health_chip(ui, chip_rect, value);
        }
    }

    fn render_stream_support_plugin_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        panel_width: f32,
    ) {
        let status_node = node.find("plugins:stream-support:status");
        let actions_node = node.find("plugins:stream-support:actions");
        let alert_node = node.find("plugins:stream-support:alert");

        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width)
                .body_margin(egui::Margin::symmetric(12, 8))
                .body_horizontal_margin(24.0),
            |renderer, ui, _body_width| {
                if let Some(alert_node) = alert_node {
                    renderer.render_action_alert_panel(ui, alert_node, state);
                    ui.add_space(8.0);
                }
                if let Some(status_node) = status_node {
                    renderer.render_stream_support_overview(ui, status_node);
                    ui.add_space(8.0);
                    renderer.render_stream_support_status_cards(ui, status_node);
                }
                if let Some(actions_node) = actions_node {
                    ui.add_space(8.0);
                    renderer.render_stream_support_plugin_actions(ui, actions_node, state);
                }
            },
        );
    }

    fn render_plex_plugin_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        panel_width: f32,
    ) {
        let status_node = node.find("plugins:plex:status");
        let servers_node = node.find("plugins:plex:servers");
        let actions_node = node.find("plugins:plex:actions");

        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width)
                .body_margin(egui::Margin::symmetric(12, 8))
                .body_horizontal_margin(24.0),
            |renderer, ui, _body_width| {
                if let Some(status_node) = status_node {
                    renderer.render_plex_overview(ui, status_node);
                    ui.add_space(8.0);
                    renderer.render_plex_status_cards(ui, status_node);
                }
                if let Some(servers_node) = servers_node {
                    ui.add_space(8.0);
                    renderer.render_plex_server_cards(ui, servers_node, state);
                }
                if let Some(actions_node) = actions_node {
                    ui.add_space(8.0);
                    renderer.render_plugin_action_buttons(ui, actions_node, state);
                }
            },
        );
    }

    fn render_media_matching_plugin_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        panel_width: f32,
    ) {
        let status_node = node.find("plugins:media-matching:status");
        let settings_node = node.find("plugins:media-matching:settings");
        let remediation_node = node.find("plugins:media-matching:remediation");
        let actions_node = node.find("plugins:media-matching:actions");

        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width)
                .body_margin(egui::Margin::symmetric(12, 8))
                .body_horizontal_margin(24.0),
            |renderer, ui, _body_width| {
                if let Some(status_node) = status_node {
                    renderer.render_media_matching_overview(ui, status_node);
                    ui.add_space(8.0);
                    renderer.render_media_matching_status_cards(ui, status_node);
                }
                if let Some(remediation_node) = remediation_node {
                    ui.add_space(8.0);
                    renderer.render_plugin_status_cards(ui, remediation_node, &[]);
                }
                if let Some(settings_node) = settings_node {
                    ui.add_space(8.0);
                    renderer.render_media_matching_settings_cards(ui, settings_node, state);
                }
                if let Some(actions_node) = actions_node {
                    ui.add_space(8.0);
                    renderer.render_plugin_action_buttons(ui, actions_node, state);
                }
            },
        );
    }

    fn render_stream_support_overview(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_overview(
            ui,
            status_node,
            "plugins:stream-support:title",
            "plugins:stream-support:summary",
            "plugins:stream-support:health",
            "Stream helper status",
        );
    }

    fn render_plex_overview(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_overview(
            ui,
            status_node,
            "plugins:plex:title",
            "plugins:plex:summary",
            "plugins:plex:health",
            "Plex watch sync",
        );
    }

    fn render_media_matching_overview(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_overview(
            ui,
            status_node,
            "plugins:media-matching:title",
            "plugins:media-matching:summary",
            "plugins:media-matching:health",
            "Media matching status",
        );
    }

    fn render_plugin_overview(
        &self,
        ui: &mut egui::Ui,
        status_node: &GuiWidgetNode,
        title_id: &str,
        summary_id: &str,
        health_id: &str,
        default_title: &str,
    ) {
        let title = status_node
            .find(title_id)
            .map(Self::display_status_value)
            .unwrap_or_else(|| default_title.to_owned());
        let summary = status_node
            .find(summary_id)
            .map(Self::display_status_value)
            .unwrap_or_default();
        let health = status_node
            .find(health_id)
            .map(Self::display_status_value)
            .unwrap_or_default();

        let palette = Self::palette_for_ui(ui);
        let available_width = Self::visible_available_width(ui);
        egui::Frame::new()
            .fill(palette.surface_muted)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                let inner_width = Self::width_inside_horizontal_margin(available_width, 24.0);
                ui.set_width(inner_width);
                ui.set_max_width(inner_width);
                if inner_width >= 560.0 {
                    ui.horizontal_top(|ui| {
                        let chip_width = 96.0;
                        let text_width = (inner_width - chip_width - 12.0).max(0.0);
                        Self::allocate_plugin_width(ui, text_width, |ui| {
                            ui.label(
                                egui::RichText::new(title.clone())
                                    .strong()
                                    .color(palette.neutral_text),
                            );
                            if !summary.is_empty() {
                                ui.label(
                                    egui::RichText::new(summary.clone())
                                        .small()
                                        .color(palette.muted_text),
                                );
                            }
                        });
                        if !health.is_empty() {
                            ui.add_space(12.0);
                            let (rect, _response) = ui.allocate_exact_size(
                                egui::vec2(chip_width, 24.0),
                                egui::Sense::hover(),
                            );
                            Self::paint_stream_support_health_chip(ui, rect, &health);
                        }
                    });
                } else {
                    ui.label(
                        egui::RichText::new(title)
                            .strong()
                            .color(palette.neutral_text),
                    );
                    if !summary.is_empty() {
                        ui.label(
                            egui::RichText::new(summary)
                                .small()
                                .color(palette.muted_text),
                        );
                    }
                    if !health.is_empty() {
                        ui.add_space(6.0);
                        let (rect, _response) =
                            ui.allocate_exact_size(egui::vec2(96.0, 24.0), egui::Sense::hover());
                        Self::paint_stream_support_health_chip(ui, rect, &health);
                    }
                }
            });
    }

    fn render_stream_support_status_cards(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_status_cards(
            ui,
            status_node,
            &[
                "plugins:stream-support:title",
                "plugins:stream-support:summary",
                "plugins:stream-support:health",
            ],
        );
    }

    fn render_plex_status_cards(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_status_cards(
            ui,
            status_node,
            &[
                "plugins:plex:title",
                "plugins:plex:summary",
                "plugins:plex:health",
            ],
        );
    }

    fn render_media_matching_status_cards(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        self.render_plugin_status_cards(
            ui,
            status_node,
            &[
                "plugins:media-matching:title",
                "plugins:media-matching:summary",
                "plugins:media-matching:health",
            ],
        );
    }

    fn render_plugin_status_cards(
        &self,
        ui: &mut egui::Ui,
        status_node: &GuiWidgetNode,
        hidden_ids: &[&str],
    ) {
        let items = status_node
            .children
            .iter()
            .filter(|child| !hidden_ids.contains(&child.id.as_str()))
            .collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }

        let gap = 8.0;
        let available_width = Self::visible_available_width(ui);
        let columns: usize = if available_width >= 720.0 { 2 } else { 1 };
        let card_width = ((available_width - (gap * (columns.saturating_sub(1)) as f32))
            / columns as f32)
            .max(0.0);
        for (row_index, chunk) in items.chunks(columns).enumerate() {
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                for child in chunk {
                    Self::allocate_plugin_width(ui, card_width, |ui| {
                        self.render_plugin_status_card(ui, child, card_width);
                    });
                }
            });
            if row_index + 1 < items.len().div_ceil(columns) {
                ui.add_space(gap);
            }
        }
    }

    fn render_plugin_status_card(&self, ui: &mut egui::Ui, node: &GuiWidgetNode, card_width: f32) {
        let palette = Self::palette_for_ui(ui);
        egui::Frame::new()
            .fill(palette.surface_muted.gamma_multiply(0.84))
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                let inner_width = Self::width_inside_horizontal_margin(card_width, 20.0);
                ui.set_width(inner_width);
                ui.set_max_width(inner_width);
                ui.set_min_height(40.0);
                ui.label(
                    egui::RichText::new(&node.label)
                        .small()
                        .strong()
                        .color(palette.muted_text),
                );
                let value = Self::display_status_value(node);
                let text_color = palette.neutral_text;
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                let (display_value, truncated) = Self::truncate_single_line_text_for_width(
                    ui,
                    &value,
                    font_id.clone(),
                    text_color,
                    inner_width,
                );
                let response = ui.add(
                    egui::Label::new(egui::RichText::new(display_value).color(text_color))
                        .selectable(false),
                );
                if truncated {
                    response.on_hover_text(value);
                }
            });
    }

    fn render_media_matching_settings_cards(
        &mut self,
        ui: &mut egui::Ui,
        settings_node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let toggle_nodes = settings_node
            .children
            .iter()
            .filter(|child| matches!(child.kind, GuiWidgetKind::Checkbox))
            .collect::<Vec<_>>();
        let policy_nodes = settings_node
            .children
            .iter()
            .filter(|child| child.id.starts_with("plugins:media-matching:policy:"))
            .collect::<Vec<_>>();

        if !toggle_nodes.is_empty() {
            self.render_media_matching_toggle_cards(ui, &toggle_nodes, state);
        }
        if !policy_nodes.is_empty() {
            if !toggle_nodes.is_empty() {
                ui.add_space(8.0);
            }
            self.render_media_matching_policy_card(ui, &policy_nodes, state);
        }
    }

    fn render_media_matching_toggle_cards(
        &mut self,
        ui: &mut egui::Ui,
        toggle_nodes: &[&GuiWidgetNode],
        state: &SorotteGuiShellAppState,
    ) {
        let gap = 8.0;
        let available_width = Self::visible_available_width(ui);
        let columns: usize = if available_width >= 720.0 { 2 } else { 1 };
        let card_width = ((available_width - (gap * columns.saturating_sub(1) as f32))
            / columns as f32)
            .max(0.0);
        let row_count = toggle_nodes.len().div_ceil(columns);
        for (row_index, chunk) in toggle_nodes.chunks(columns).enumerate() {
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                for child in chunk {
                    Self::allocate_plugin_width(ui, card_width, |ui| {
                        self.render_media_matching_setting_card(ui, child, state, card_width);
                    });
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(gap);
            }
        }
    }

    fn render_media_matching_setting_card(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        card_width: f32,
    ) {
        let palette = Self::palette_for_ui(ui);
        egui::Frame::new()
            .fill(palette.surface_muted.gamma_multiply(0.84))
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                let inner_width = Self::width_inside_horizontal_margin(card_width, 20.0);
                ui.set_width(inner_width);
                ui.set_max_width(inner_width);
                ui.set_min_height(32.0);
                self.render_field_control(ui, node, state, false);
            });
    }

    fn render_media_matching_policy_card(
        &mut self,
        ui: &mut egui::Ui,
        policy_nodes: &[&GuiWidgetNode],
        state: &SorotteGuiShellAppState,
    ) {
        let palette = Self::palette_for_ui(ui);
        let available_width = Self::visible_available_width(ui);
        egui::Frame::new()
            .fill(palette.surface_muted.gamma_multiply(0.84))
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                let inner_width = Self::width_inside_horizontal_margin(available_width, 20.0);
                ui.set_width(inner_width);
                ui.set_max_width(inner_width);
                ui.label(
                    egui::RichText::new("Autoplay Policy")
                        .small()
                        .strong()
                        .color(palette.muted_text),
                );
                if let Some(summary) = policy_nodes
                    .iter()
                    .find_map(|node| node.value.as_ref())
                    .filter(|summary| !summary.is_empty())
                {
                    let font_id = egui::TextStyle::Small.resolve(ui.style());
                    let (display_summary, truncated) = Self::truncate_single_line_text_for_width(
                        ui,
                        summary,
                        font_id.clone(),
                        palette.muted_text,
                        inner_width,
                    );
                    let response = ui.label(
                        egui::RichText::new(display_summary)
                            .small()
                            .color(palette.muted_text),
                    );
                    if truncated {
                        response.on_hover_text(summary.clone());
                    }
                    ui.add_space(4.0);
                } else {
                    ui.add_space(4.0);
                }
                self.render_media_matching_policy_buttons(ui, policy_nodes, state);
            });
    }

    fn render_media_matching_policy_buttons(
        &mut self,
        ui: &mut egui::Ui,
        policy_nodes: &[&GuiWidgetNode],
        state: &SorotteGuiShellAppState,
    ) {
        let available_width = Self::visible_available_width(ui);
        let gap = 8.0;
        let columns: usize = if available_width >= 520.0 { 2 } else { 1 };
        let button_width = ((available_width - (gap * columns.saturating_sub(1) as f32))
            / columns as f32)
            .max(0.0);
        let row_count = policy_nodes.len().div_ceil(columns);
        for (row_index, chunk) in policy_nodes.chunks(columns).enumerate() {
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                for child in chunk {
                    Self::allocate_plugin_width(ui, button_width, |ui| {
                        self.render_media_matching_policy_button(ui, child, state, button_width);
                    });
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(4.0);
            }
        }
    }

    fn render_media_matching_policy_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        button_width: f32,
    ) {
        let button_height = 36.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(button_width, button_height),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &node.label)
        });
        let response = Self::attach_node_tooltip(response, node);
        if node.enabled && response.clicked() {
            self.handle_button_node_click(state, node);
        }

        let palette = Self::palette_for_ui(ui);
        let visuals = ui.visuals();
        let hovered = response.hovered();
        let fill = if !node.enabled {
            visuals.widgets.inactive.bg_fill.gamma_multiply(0.55)
        } else if node.selected {
            palette.info_bg
        } else if hovered {
            palette.surface_muted
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let stroke = if !node.enabled {
            egui::Stroke::new(
                1.0,
                visuals
                    .widgets
                    .inactive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.65),
            )
        } else if node.selected {
            egui::Stroke::new(1.5, palette.info_border)
        } else if hovered {
            egui::Stroke::new(1.0, palette.primary)
        } else {
            egui::Stroke::new(1.0, palette.border)
        };
        let text_color = if !node.enabled {
            visuals.weak_text_color()
        } else if node.selected {
            palette.info_text
        } else {
            palette.neutral_text
        };
        let button_rect = rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter()
            .rect(button_rect, 5, fill, stroke, egui::StrokeKind::Inside);
        let text_width = (button_rect.width() - 22.0).max(0.0);
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let (display_label, truncated) = Self::truncate_single_line_text_for_width(
            ui,
            &node.label,
            font_id.clone(),
            text_color,
            text_width,
        );
        let galley = ui
            .painter()
            .layout_no_wrap(display_label, font_id, text_color);
        ui.painter().with_clip_rect(button_rect).galley(
            egui::pos2(
                button_rect.left() + 11.0,
                button_rect.center().y - (galley.size().y * 0.5),
            ),
            galley,
            text_color,
        );
        if truncated {
            response.on_hover_text(node.label.clone());
        }
    }

    fn render_stream_support_plugin_actions(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        self.render_plugin_action_buttons(ui, node, state);
    }

    fn render_plugin_action_buttons(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let available_width = Self::visible_available_width(ui);
        let gap = 8.0;
        let target_button_width = 176.0;
        let buttons_per_row = if available_width < 360.0 {
            1
        } else {
            ((available_width + gap) / (target_button_width + gap))
                .floor()
                .max(1.0) as usize
        };
        let button_width = if buttons_per_row == 1 && available_width < 360.0 {
            available_width
        } else {
            ((available_width - (gap * buttons_per_row.saturating_sub(1) as f32))
                / buttons_per_row as f32)
                .clamp(150.0, 220.0)
        };
        let row_count = node.children.len().div_ceil(buttons_per_row);
        for (row_index, chunk) in node.children.chunks(buttons_per_row).enumerate() {
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                for child in chunk {
                    Self::allocate_plugin_width(ui, button_width, |ui| {
                        self.render_button_like(ui, child, state);
                    });
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(4.0);
            }
        }
    }

    fn render_plex_server_cards(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        if node.children.is_empty() {
            return;
        }
        let available_width = Self::visible_available_width(ui);
        let gap = 8.0;
        let columns: usize = if available_width >= 720.0 { 2 } else { 1 };
        let card_width = ((available_width - (gap * columns.saturating_sub(1) as f32))
            / columns as f32)
            .max(0.0);
        let row_count = node.children.len().div_ceil(columns);
        for (row_index, chunk) in node.children.chunks(columns).enumerate() {
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                for child in chunk {
                    Self::allocate_plugin_width(ui, card_width, |ui| {
                        self.render_plex_server_card(ui, child, state, card_width);
                    });
                }
            });
            if row_index + 1 < row_count {
                ui.add_space(gap);
            }
        }
    }

    fn render_plex_server_card(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        card_width: f32,
    ) {
        let palette = Self::palette_for_ui(ui);
        let card_height = 60.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(card_width, card_height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &node.label)
        });
        let response = Self::attach_node_tooltip(response, node);
        if response.clicked() {
            self.handle_button_node_click(state, node);
        }

        let fill = if node.selected {
            palette.info_bg
        } else if response.hovered() {
            palette.surface_muted
        } else {
            palette.surface_muted.gamma_multiply(0.84)
        };
        let stroke = if node.selected {
            egui::Stroke::new(1.5, palette.info_border)
        } else if response.hovered() {
            egui::Stroke::new(1.0, palette.primary)
        } else {
            egui::Stroke::new(1.0, palette.border)
        };
        let card_rect = rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter()
            .rect(card_rect, 5, fill, stroke, egui::StrokeKind::Inside);
        if node.selected {
            let stripe_rect =
                egui::Rect::from_min_max(card_rect.left_top(), card_rect.left_bottom())
                    .expand2(egui::vec2(2.0, 0.0))
                    .intersect(card_rect);
            ui.painter()
                .rect_filled(stripe_rect, 5, palette.info_border);
        }

        let content_rect = card_rect.shrink2(egui::vec2(12.0, 8.0));
        let server_row = Self::plex_server_row_for_node(state, node);
        let reachability = server_row
            .map(|server| server.reachability.label())
            .unwrap_or("unknown");
        let server_scope = server_row
            .map(|server| {
                if server.has_local_connection {
                    "local"
                } else if server.owned {
                    "owned"
                } else {
                    "shared"
                }
            })
            .unwrap_or("owned");
        let reachability_chip_width = 86.0;
        let kind_chip_width = 66.0;
        let chip_gap = 6.0;
        let show_kind_chip = content_rect.width() >= 300.0;
        let reachability_chip_rect = egui::Rect::from_min_size(
            egui::pos2(
                content_rect.right() - reachability_chip_width,
                content_rect.top(),
            ),
            egui::vec2(reachability_chip_width, 22.0),
        );
        let text_right = if show_kind_chip {
            let kind_chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    reachability_chip_rect.left() - kind_chip_width - chip_gap,
                    content_rect.top(),
                ),
                egui::vec2(kind_chip_width, 22.0),
            );
            Self::paint_stream_support_health_chip(ui, kind_chip_rect, server_scope);
            kind_chip_rect.left()
        } else {
            reachability_chip_rect.left()
        };
        Self::paint_stream_support_health_chip(ui, reachability_chip_rect, reachability);
        let selected_icon_width = if node.selected { 24.0 } else { 0.0 };
        if node.selected {
            let check_rect = egui::Rect::from_center_size(
                egui::pos2(content_rect.left() + 9.0, content_rect.top() + 10.5),
                egui::vec2(18.0, 18.0),
            );
            Self::paint_selected_check(ui, check_rect);
        }

        let text_left = content_rect.left() + selected_icon_width;
        let text_width = (text_right - text_left - 8.0).max(0.0);
        let title_color = if node.selected {
            palette.info_text
        } else {
            palette.neutral_text
        };
        let title_font = egui::TextStyle::Button.resolve(ui.style());
        let (display_title, title_truncated) = Self::truncate_single_line_text_for_width(
            ui,
            &node.label,
            title_font.clone(),
            title_color,
            text_width,
        );
        let title_galley = ui
            .painter()
            .layout_no_wrap(display_title, title_font, title_color);
        ui.painter().with_clip_rect(content_rect).galley(
            egui::pos2(text_left, content_rect.top()),
            title_galley,
            title_color,
        );

        let uri = node.value.as_deref().unwrap_or_default();
        if !uri.is_empty() {
            let uri_font = egui::TextStyle::Small.resolve(ui.style());
            let uri_color = palette.muted_text;
            let (display_uri, uri_truncated) = Self::truncate_single_line_text_for_width(
                ui,
                uri,
                uri_font.clone(),
                uri_color,
                text_width,
            );
            let uri_galley = ui
                .painter()
                .layout_no_wrap(display_uri, uri_font, uri_color);
            ui.painter().with_clip_rect(content_rect).galley(
                egui::pos2(text_left, content_rect.top() + 22.0),
                uri_galley,
                uri_color,
            );
            if uri_truncated || title_truncated {
                response.on_hover_text(format!("{}\n{}", node.label, uri));
            }
        } else if title_truncated {
            response.on_hover_text(node.label.clone());
        }
    }

    fn plex_server_row_for_node<'a>(
        state: &'a SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<&'a super::super::shell_state::GuiPlexServerRow> {
        let index = node
            .id
            .strip_prefix("plugins:plex:server:")?
            .parse::<usize>()
            .ok()?;
        state.plex.servers.get(index)
    }

    fn paint_selected_check(ui: &egui::Ui, rect: egui::Rect) {
        let palette = Self::palette_for_ui(ui);
        ui.painter()
            .circle_filled(rect.center(), rect.width() * 0.5, palette.info_border);
        let stroke = egui::Stroke::new(1.8, palette.primary_text);
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 4.5, rect.center().y),
                egui::pos2(rect.left() + 7.5, rect.bottom() - 5.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 7.5, rect.bottom() - 5.0),
                egui::pos2(rect.right() - 4.0, rect.top() + 5.0),
            ],
            stroke,
        );
    }

    fn paint_stream_support_health_chip(ui: &egui::Ui, rect: egui::Rect, value: &str) {
        let palette = Self::palette_for_ui(ui);
        let normalized = value.to_ascii_lowercase();
        let (fill, stroke, text) = if normalized.contains("healthy")
            || normalized.contains("ready")
            || normalized.contains("connected")
            || normalized.contains("enabled")
            || normalized.contains("syncing")
            || normalized.contains("reachable")
            || normalized.contains("local")
            || normalized.contains("owned")
        {
            (
                palette.success_bg,
                palette.success_border,
                palette.success_text,
            )
        } else if normalized.contains("broken")
            || normalized.contains("error")
            || normalized.contains("offline")
        {
            (
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(72, 37, 35)
                } else {
                    egui::Color32::from_rgb(255, 240, 239)
                },
                palette.danger,
                palette.danger,
            )
        } else {
            (
                palette.warning_bg,
                palette.warning_border,
                palette.warning_text,
            )
        };
        let rect = rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            12,
            fill,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        let label = value.to_ascii_uppercase();
        let font_id = egui::TextStyle::Small.resolve(ui.style());
        let (display_label, _) = Self::truncate_single_line_text_for_width(
            ui,
            &label,
            font_id.clone(),
            text,
            (rect.width() - 14.0).max(0.0),
        );
        let galley = ui.painter().layout_no_wrap(display_label, font_id, text);
        ui.painter().galley(
            egui::pos2(
                rect.center().x - (galley.size().x * 0.5),
                rect.center().y - (galley.size().y * 0.5),
            ),
            galley,
            text,
        );
    }
}
