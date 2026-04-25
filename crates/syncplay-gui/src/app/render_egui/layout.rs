use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::{GuiLayoutMode, GuiWidgetKind, GuiWidgetNode};
use super::GuiWidgetEguiRenderer;

impl GuiWidgetEguiRenderer {
    pub(super) fn render_room_dashboard(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(summary_column) = node.find("main-window:summary-column") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(browser) = node.find("main-window:browser") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(playlist_column) = node.find("main-window:playlist-column") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(chat_panel) = node.find("main-window:chat-panel") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(session_panel) = summary_column.find("main-window:connection") else {
            self.render_layout(ui, node, state);
            return;
        };
        let Some(controls_panel) = summary_column.find("main-window:controls") else {
            self.render_layout(ui, node, state);
            return;
        };

        let available_width = ui.available_width().clamp(0.0, 1420.0);
        ui.set_max_width(available_width);
        let gap = 12.0;
        if available_width < 820.0 {
            self.render_layout(ui, node, state);
            return;
        }

        if available_width < 1180.0 {
            let column_width = ((available_width - gap) * 0.5).max(0.0);
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                Self::allocate_fixed_width(ui, column_width, |ui| {
                    self.render_node(ui, session_panel, state);
                });
                Self::allocate_fixed_width(ui, column_width, |ui| {
                    self.render_node(ui, browser, state);
                });
            });
            ui.add_space(gap);
            ui.horizontal_top(|ui| {
                let mut spacing = ui.spacing().item_spacing;
                spacing.x = gap;
                ui.spacing_mut().item_spacing = spacing;
                Self::allocate_fixed_width(ui, column_width, |ui| {
                    self.render_node(ui, controls_panel, state);
                });
                Self::allocate_fixed_width(ui, column_width, |ui| {
                    self.render_node(ui, playlist_column, state);
                });
            });
            ui.add_space(gap);
            self.render_node(ui, chat_panel, state);
            return;
        }

        let summary_width = (available_width * 0.30).clamp(330.0, 390.0);
        let work_width = (available_width - summary_width - gap).max(0.0);
        let work_column_width = ((work_width - gap) * 0.5).max(240.0);
        let chat_width = work_width.max(0.0);

        ui.horizontal_top(|ui| {
            let mut spacing = ui.spacing().item_spacing;
            spacing.x = gap;
            ui.spacing_mut().item_spacing = spacing;

            Self::allocate_fixed_width(ui, summary_width, |ui| {
                self.render_node(ui, session_panel, state);
                ui.add_space(gap);
                self.render_node(ui, controls_panel, state);
            });

            Self::allocate_fixed_width(ui, work_width, |ui| {
                ui.horizontal_top(|ui| {
                    let mut spacing = ui.spacing().item_spacing;
                    spacing.x = gap;
                    ui.spacing_mut().item_spacing = spacing;
                    Self::allocate_fixed_width(ui, work_column_width, |ui| {
                        self.render_node(ui, browser, state);
                    });
                    Self::allocate_fixed_width(ui, work_column_width, |ui| {
                        self.render_node(ui, playlist_column, state);
                    });
                });
                ui.add_space(gap);
                Self::allocate_fixed_width(ui, chat_width, |ui| {
                    self.render_node(ui, chat_panel, state);
                });
            });
        });
    }

    fn allocate_fixed_width(
        ui: &mut egui::Ui,
        width: f32,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        ui.allocate_ui_with_layout(
            egui::vec2(width.max(0.0), 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(width.max(0.0));
                ui.set_max_width(width.max(0.0));
                add_contents(ui);
            },
        );
    }

    pub(super) fn render_layout(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(layout_mode) = node.layout_mode else {
            for child in &node.children {
                self.render_node(ui, child, state);
            }
            return;
        };
        match layout_mode {
            GuiLayoutMode::Stack => {
                for child in &node.children {
                    self.render_node(ui, child, state);
                }
            }
            GuiLayoutMode::ResponsiveColumns {
                min_column_width,
                max_columns,
            } => {
                let plan = Self::plan_responsive_columns(
                    ui.available_width(),
                    12.0,
                    min_column_width,
                    max_columns,
                    node.children.iter().map(|child| child.column_span),
                );
                for (row_index, row) in plan.rows.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 0.0;
                        ui.spacing_mut().item_spacing = spacing;
                        let mut current_column = 0usize;
                        for entry in row {
                            if entry.column > current_column {
                                let spacer_columns = entry.column - current_column;
                                let spacer_width = (spacer_columns as f32 * plan.column_width)
                                    + ((spacer_columns.saturating_sub(1)) as f32 * 12.0);
                                ui.add_space(spacer_width + 12.0);
                            }
                            let child_width = (entry.span as f32 * plan.column_width)
                                + ((entry.span.saturating_sub(1)) as f32 * 12.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(child_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(child_width);
                                    self.render_node(ui, &node.children[entry.child_index], state);
                                },
                            );
                            current_column = entry.column + entry.span;
                            if current_column < plan.column_count {
                                ui.add_space(12.0);
                            }
                        }
                    });
                    if row_index + 1 < plan.row_count {
                        ui.add_space(12.0);
                    }
                }
            }
            GuiLayoutMode::TabStrip { min_tab_width } => {
                let edge_padding = 8.0;
                let gap = 8.0;
                let plan = Self::plan_responsive_columns(
                    (ui.available_width() - (edge_padding * 2.0)).max(0.0),
                    gap,
                    min_tab_width,
                    node.children.len().max(1),
                    node.children.iter().map(|_| 1usize),
                );
                for (row_index, row) in plan.rows.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 0.0;
                        spacing.y = gap;
                        ui.spacing_mut().item_spacing = spacing;
                        ui.add_space(edge_padding);
                        for (entry_index, entry) in row.iter().enumerate() {
                            let child_width = plan.column_width.max(0.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(child_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(child_width);
                                    self.render_tab_button(
                                        ui,
                                        &node.children[entry.child_index],
                                        state,
                                        child_width,
                                    );
                                },
                            );
                            if entry_index + 1 < row.len() {
                                ui.add_space(gap);
                            }
                        }
                        ui.add_space(edge_padding);
                    });
                    if row_index + 1 < plan.row_count {
                        ui.add_space(gap);
                    }
                }
            }
            GuiLayoutMode::FormGrid {
                label_width,
                min_field_width,
            } => {
                let stacked = (ui.available_width() - label_width) < min_field_width;
                for child in &node.children {
                    if stacked {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&child.label).strong());
                            self.render_field_control(ui, child, state, true);
                        });
                    } else {
                        ui.horizontal_top(|ui| {
                            let label_height = ui
                                .spacing()
                                .interact_size
                                .y
                                .max(ui.text_style_height(&egui::TextStyle::Body));
                            let (label_rect, _) = ui.allocate_exact_size(
                                egui::vec2(label_width, label_height),
                                egui::Sense::hover(),
                            );
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(label_rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                |ui| {
                                    ui.label(egui::RichText::new(&child.label).strong());
                                },
                            );
                            self.render_field_control(ui, child, state, true);
                        });
                    }
                }
            }
            GuiLayoutMode::KeyValueGrid { min_pair_width } => {
                let plan = Self::plan_responsive_columns(
                    ui.available_width(),
                    12.0,
                    min_pair_width,
                    2,
                    node.children.iter().map(|_| 1usize),
                );
                for (row_index, row) in plan.rows.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 0.0;
                        ui.spacing_mut().item_spacing = spacing;
                        for (entry_index, entry) in row.iter().enumerate() {
                            let child_width = plan.column_width;
                            ui.allocate_ui_with_layout(
                                egui::vec2(child_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(child_width);
                                    self.render_key_value_item(
                                        ui,
                                        &node.children[entry.child_index],
                                        state,
                                    );
                                },
                            );
                            if entry_index + 1 < row.len() {
                                ui.add_space(12.0);
                            }
                        }
                    });
                    if row_index + 1 < plan.row_count {
                        ui.add_space(8.0);
                    }
                }
            }
            GuiLayoutMode::ButtonWrap { min_button_width } => {
                let buttons_per_row = ((ui.available_width() + 12.0) / (min_button_width + 12.0))
                    .floor()
                    .max(1.0) as usize;
                for chunk in node.children.chunks(buttons_per_row) {
                    let row_button_width = ((ui.available_width()
                        - (12.0 * (chunk.len().saturating_sub(1)) as f32))
                        / chunk.len() as f32)
                        .max(0.0);
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 12.0;
                        ui.spacing_mut().item_spacing = spacing;
                        for child in chunk {
                            ui.allocate_ui_with_layout(
                                egui::vec2(row_button_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(row_button_width);
                                    self.render_button_like(ui, child, state);
                                },
                            );
                        }
                    });
                    ui.add_space(8.0);
                }
            }
            GuiLayoutMode::CompactButtonWrap {
                button_width,
                button_height,
                gap,
            } => {
                let button_width = button_width.max(1.0);
                let button_height = button_height.max(1.0);
                let gap = gap.max(0.0);
                let buttons_per_row = ((ui.available_width() + gap) / (button_width + gap))
                    .floor()
                    .max(1.0) as usize;
                let row_count = node.children.len().div_ceil(buttons_per_row);
                for (row_index, chunk) in node.children.chunks(buttons_per_row).enumerate() {
                    ui.horizontal_top(|ui| {
                        let available_width = ui.available_width();
                        let row_width = (button_width * chunk.len() as f32)
                            + (gap * chunk.len().saturating_sub(1) as f32);
                        let side_space = ((available_width - row_width).max(0.0)) * 0.5;
                        if side_space > 0.0 {
                            ui.add_space(side_space);
                        }
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = gap;
                        ui.spacing_mut().item_spacing = spacing;
                        for child in chunk {
                            ui.allocate_ui_with_layout(
                                egui::vec2(button_width, button_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_min_size(egui::vec2(button_width, button_height));
                                    self.render_button_like(ui, child, state);
                                },
                            );
                        }
                    });
                    if row_index + 1 < row_count {
                        ui.add_space(gap);
                    }
                }
            }
        }
    }

    pub(super) fn render_leaf(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::Layout => {}
            GuiWidgetKind::TextInput
            | GuiWidgetKind::TextArea
            | GuiWidgetKind::PasswordInput
            | GuiWidgetKind::NumericInput => {
                if node.kind == GuiWidgetKind::TextArea {
                    self.render_text_area(ui, node, state);
                } else {
                    self.render_text_input(ui, node, state);
                }
            }
            GuiWidgetKind::Select => self.render_select(ui, node, state),
            GuiWidgetKind::Checkbox => {
                let mut checked = matches!(node.value.as_deref(), Some("yes" | "true"));
                let response =
                    ui.add_enabled(node.enabled, egui::Checkbox::new(&mut checked, &node.label));
                if response.changed()
                    && let Some(action) = Self::action_for_checkbox_node(state, node, checked)
                {
                    self.actions.push(action);
                }
            }
            GuiWidgetKind::Button => {
                self.render_button_like(ui, node, state);
            }
            GuiWidgetKind::ListItem => {
                ui.add_enabled_ui(node.enabled, |ui| {
                    if ui
                        .selectable_label(node.selected, Self::display_text(node))
                        .clicked()
                        && let Some(action) = Self::action_for_list_item_node(node)
                    {
                        self.actions.push(action);
                    }
                });
            }
            GuiWidgetKind::ReadOnly | GuiWidgetKind::Status => {
                if Self::should_render_combined_status_label(node) {
                    ui.label(Self::display_text(node));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = spacing.x.max(4.0);
                        ui.spacing_mut().item_spacing = spacing;
                        ui.label(egui::RichText::new(format!("{}:", node.label)).strong());
                        ui.label(Self::display_status_rich_text(ui, node));
                    });
                }
            }
            GuiWidgetKind::Panel | GuiWidgetKind::List => {}
        }
    }

    fn render_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        tab_width: f32,
    ) {
        let mut clicked = false;
        ui.add_enabled_ui(node.enabled, |ui| {
            clicked = ui
                .add_sized(
                    [tab_width.max(0.0), 0.0],
                    egui::Button::new(Self::display_text(node)).selected(node.selected),
                )
                .clicked();
        });
        if clicked {
            if let Some(actions) = Self::direct_menu_actions(state, node) {
                self.actions.extend(actions);
            } else {
                self.actions
                    .extend(Self::actions_for_clicked_button(state, node));
            }
        }
    }
}
