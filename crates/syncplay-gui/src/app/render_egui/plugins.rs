use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::GuiWidgetNode;
use super::{GuiPanelShellOptions, GuiWidgetEguiRenderer};

impl GuiWidgetEguiRenderer {
    const PLUGINS_GAP: f32 = 12.0;
    const PLUGINS_STACK_BREAKPOINT: f32 = 760.0;

    pub(super) fn render_plugins_surface(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(plugin_list) = node.find("plugins:list") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(stream_support) = node.find("plugins:stream-support") else {
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
                            self.render_stream_support_plugin_panel(
                                ui,
                                stream_support,
                                state,
                                detail_width,
                            );
                        });
                    });
                } else {
                    self.render_plugins_list_panel(ui, plugin_list, state, content_width);
                    ui.add_space(Self::PLUGINS_GAP);
                    self.render_stream_support_plugin_panel(
                        ui,
                        stream_support,
                        state,
                        content_width,
                    );
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
        state: &SyncplayGuiShellAppState,
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

    fn render_plugin_list_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        _state: &SyncplayGuiShellAppState,
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
        state: &SyncplayGuiShellAppState,
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

    fn render_stream_support_overview(&self, ui: &mut egui::Ui, status_node: &GuiWidgetNode) {
        let title = status_node
            .find("plugins:stream-support:title")
            .map(Self::display_status_value)
            .unwrap_or_else(|| "Stream helper status".to_owned());
        let summary = status_node
            .find("plugins:stream-support:summary")
            .map(Self::display_status_value)
            .unwrap_or_default();
        let health = status_node
            .find("plugins:stream-support:health")
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
        let items = status_node
            .children
            .iter()
            .filter(|child| {
                !matches!(
                    child.id.as_str(),
                    "plugins:stream-support:title"
                        | "plugins:stream-support:summary"
                        | "plugins:stream-support:health"
                )
            })
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
                        self.render_stream_support_status_card(ui, child, card_width);
                    });
                }
            });
            if row_index + 1 < items.len().div_ceil(columns) {
                ui.add_space(gap);
            }
        }
    }

    fn render_stream_support_status_card(
        &self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        card_width: f32,
    ) {
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

    fn render_stream_support_plugin_actions(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
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

    fn paint_stream_support_health_chip(ui: &egui::Ui, rect: egui::Rect, value: &str) {
        let palette = Self::palette_for_ui(ui);
        let normalized = value.to_ascii_lowercase();
        let (fill, stroke, text) = if normalized.contains("healthy") {
            (
                palette.success_bg,
                palette.success_border,
                palette.success_text,
            )
        } else if normalized.contains("broken") || normalized.contains("error") {
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
