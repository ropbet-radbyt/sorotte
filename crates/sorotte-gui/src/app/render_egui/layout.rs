use eframe::egui;

use super::super::shell_state::SorotteGuiShellAppState;
use super::super::widget_tree::{GuiLayoutMode, GuiWidgetKind, GuiWidgetNode};
use super::{GuiRoomDashboardLayout, GuiWidgetEguiRenderer};

impl GuiWidgetEguiRenderer {
    pub(super) fn render_room_dashboard(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let Some(summary_column) = node.find("main-window:summary-column") else {
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
        let viewport_width = Self::visible_available_width(ui);
        let available_width = Self::room_dashboard_content_width(viewport_width);
        let available_height = Self::visible_available_height(ui);
        let gap = 12.0;
        let playlist_editor_active = playlist_column.find("main-window:playlist-edit").is_some()
            || playlist_column
                .find("main-window:playlist-url-edit")
                .is_some()
            || playlist_column
                .find("main-window:playlist-plex-search")
                .is_some();
        match Self::room_dashboard_layout_for_width(available_width) {
            GuiRoomDashboardLayout::Narrow => {
                let column_width = available_width.clamp(0.0, 720.0);
                let stacked_gap = if available_width < 560.0 { 48.0 } else { gap };
                Self::allocate_centered_row(ui, column_width, |ui| {
                    Self::allocate_fixed_width(ui, column_width, |ui| {
                        self.render_node(ui, session_panel, state);
                        ui.add_space(stacked_gap);
                        self.render_node(ui, playlist_column, state);
                        ui.add_space(stacked_gap);
                        self.render_node(ui, chat_panel, state);
                    });
                });
            }
            GuiRoomDashboardLayout::Medium => {
                let column_width = ((available_width - gap) * 0.5).clamp(320.0, 640.0);
                let row_width = (column_width * 2.0) + gap;
                let top_row_height =
                    Self::room_dashboard_top_row_height(available_height, playlist_editor_active);
                let bottom_row_height =
                    Self::room_dashboard_bottom_row_height(available_height, top_row_height);
                Self::allocate_centered_row(ui, row_width, |ui| {
                    self.render_fixed_width_row_with_min_heights(
                        ui,
                        gap,
                        [column_width, column_width],
                        [
                            Some(("main-window:connection", top_row_height)),
                            Some(("main-window:playlist-surface", top_row_height)),
                        ],
                        |renderer, ui, index| match index {
                            0 => renderer.render_node(ui, session_panel, state),
                            1 => renderer.render_node(ui, playlist_column, state),
                            _ => {}
                        },
                    );
                });
                ui.add_space(gap);
                Self::allocate_centered_row(ui, row_width, |ui| {
                    self.push_node_min_height_override("main-window:chat-panel", bottom_row_height);
                    self.render_node(ui, chat_panel, state);
                    self.pop_node_min_height_override();
                });
            }
            GuiRoomDashboardLayout::Wide => {
                let row_width = available_width.clamp(0.0, 1600.0);
                let room_panel_width = (row_width * 0.46)
                    .clamp(420.0, 720.0)
                    .min((row_width - gap - 360.0).max(0.0));
                let playlist_panel_width = (row_width - room_panel_width - gap).max(0.0);
                let top_row_height =
                    Self::room_dashboard_top_row_height(available_height, playlist_editor_active);
                let bottom_row_height =
                    Self::room_dashboard_bottom_row_height(available_height, top_row_height);

                Self::allocate_centered_row(ui, row_width, |ui| {
                    self.render_fixed_width_row_with_min_heights(
                        ui,
                        gap,
                        [room_panel_width, playlist_panel_width],
                        [
                            Some(("main-window:connection", top_row_height)),
                            Some(("main-window:playlist-surface", top_row_height)),
                        ],
                        |renderer, ui, index| match index {
                            0 => renderer.render_node(ui, session_panel, state),
                            1 => renderer.render_node(ui, playlist_column, state),
                            _ => {}
                        },
                    );
                });
                ui.add_space(gap);
                Self::allocate_centered_row(ui, row_width, |ui| {
                    self.push_node_min_height_override("main-window:chat-panel", bottom_row_height);
                    self.render_node(ui, chat_panel, state);
                    self.pop_node_min_height_override();
                });
            }
        }
    }

    #[cfg(test)]
    pub(super) fn room_dashboard_row_groups_for_width(width: f32) -> Vec<Vec<&'static str>> {
        match Self::room_dashboard_layout_for_width(width) {
            GuiRoomDashboardLayout::Narrow => vec![vec![
                "main-window:connection",
                "main-window:playlist-column",
                "main-window:chat-panel",
            ]],
            GuiRoomDashboardLayout::Medium => vec![
                vec!["main-window:connection", "main-window:playlist-column"],
                vec!["main-window:chat-panel"],
            ],
            GuiRoomDashboardLayout::Wide => vec![
                vec!["main-window:connection", "main-window:playlist-column"],
                vec!["main-window:chat-panel"],
            ],
        }
    }

    pub(super) fn room_dashboard_content_width(viewport_width: f32) -> f32 {
        (viewport_width - 24.0).max(0.0)
    }

    fn render_fixed_width_row_with_min_heights<const N: usize>(
        &mut self,
        ui: &mut egui::Ui,
        gap: f32,
        widths: [f32; N],
        min_heights: [Option<(&'static str, f32)>; N],
        mut add_contents: impl FnMut(&mut Self, &mut egui::Ui, usize),
    ) {
        ui.horizontal_top(|ui| {
            let mut spacing = ui.spacing().item_spacing;
            spacing.x = gap;
            ui.spacing_mut().item_spacing = spacing;
            for (index, width) in widths.into_iter().enumerate() {
                Self::allocate_fixed_width(ui, width, |ui| {
                    if let Some((node_id, min_height)) = min_heights[index] {
                        self.push_node_min_height_override(node_id, min_height);
                        add_contents(self, ui, index);
                        self.pop_node_min_height_override();
                    } else {
                        add_contents(self, ui, index);
                    }
                });
            }
        });
    }

    fn room_dashboard_top_row_height(available_height: f32, editor_active: bool) -> f32 {
        if editor_active {
            if available_height > 0.0 && available_height < 520.0 {
                return 400.0;
            }
            return 520.0;
        }
        if available_height > 0.0 && available_height < 460.0 {
            360.0
        } else if available_height > 0.0 && available_height < 620.0 {
            440.0
        } else {
            480.0
        }
    }

    fn room_dashboard_bottom_row_height(available_height: f32, top_row_height: f32) -> f32 {
        if available_height > 0.0 {
            (available_height - top_row_height - 24.0).clamp(220.0, 320.0)
        } else {
            260.0
        }
    }

    fn allocate_centered_row(
        ui: &mut egui::Ui,
        row_width: f32,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let available_width = Self::visible_available_width(ui);
        let row_width = row_width.min(available_width).max(0.0);
        let side_space = ((available_width - row_width) * 0.5).max(0.0);
        ui.horizontal_top(|ui| {
            if side_space > 0.0 {
                ui.add_space(side_space);
            }
            Self::allocate_fixed_width(ui, row_width, add_contents);
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

    pub(super) fn constrain_ui_width_with_clip_bleed(
        ui: &mut egui::Ui,
        width: f32,
        clip_bleed: f32,
    ) {
        let width = width.max(0.0);
        ui.set_width(width);
        ui.set_max_width(width);
        let left = ui.cursor().left();
        let clip_rect = egui::Rect::from_min_max(
            egui::pos2(left, ui.clip_rect().top()),
            egui::pos2(left + width + clip_bleed.max(0.0), ui.clip_rect().bottom()),
        );
        ui.shrink_clip_rect(clip_rect);
    }

    pub(super) fn visible_available_width(ui: &egui::Ui) -> f32 {
        let cursor_left = ui.cursor().left();
        let clip_width = (ui.clip_rect().right() - cursor_left).max(0.0);
        let max_rect_width = (ui.max_rect().right() - cursor_left).max(0.0);
        ui.available_width()
            .min(clip_width)
            .min(max_rect_width)
            .max(0.0)
    }

    pub(super) fn panel_available_width(ui: &egui::Ui) -> f32 {
        let cursor_left = ui.cursor().left();
        let max_rect_width = (ui.max_rect().right() - cursor_left).max(0.0);
        Self::visible_available_width(ui)
            .max(ui.available_width().max(0.0))
            .max(max_rect_width)
            .max(0.0)
    }

    pub(super) fn width_inside_horizontal_margin(outer_width: f32, margin_sum: f32) -> f32 {
        (outer_width - margin_sum).max(0.0)
    }

    pub(super) fn visible_available_height(ui: &egui::Ui) -> f32 {
        let cursor_top = ui.cursor().top();
        let clip_height = (ui.clip_rect().bottom() - cursor_top).max(0.0);
        let max_rect_height = (ui.max_rect().bottom() - cursor_top).max(0.0);
        ui.available_height()
            .min(clip_height)
            .min(max_rect_height)
            .max(0.0)
    }

    pub(super) fn render_layout(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
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
                    Self::visible_available_width(ui),
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
                                    ui.set_max_width(child_width);
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
                let tab_count = node.children.len().max(1);
                let available_width =
                    (Self::visible_available_width(ui) - (edge_padding * 2.0)).max(0.0);
                let compact_row_width = ((available_width
                    - (gap * tab_count.saturating_sub(1) as f32))
                    / tab_count as f32)
                    .max(0.0);
                if node.children.len() > 1 && compact_row_width >= 72.0 {
                    ui.horizontal_top(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 0.0;
                        spacing.y = gap;
                        ui.spacing_mut().item_spacing = spacing;
                        ui.add_space(edge_padding);
                        for (entry_index, child) in node.children.iter().enumerate() {
                            ui.allocate_ui_with_layout(
                                egui::vec2(compact_row_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(compact_row_width);
                                    ui.set_max_width(compact_row_width);
                                    self.render_tab_button(ui, child, state, compact_row_width);
                                },
                            );
                            if entry_index + 1 < node.children.len() {
                                ui.add_space(gap);
                            }
                        }
                        ui.add_space(edge_padding);
                    });
                    return;
                }
                let plan = Self::plan_responsive_columns(
                    available_width,
                    gap,
                    min_tab_width,
                    tab_count,
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
                                    ui.set_max_width(child_width);
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
                let previous_spacing = ui.spacing().item_spacing;
                let mut spacing = previous_spacing;
                spacing.y = 0.0;
                ui.spacing_mut().item_spacing = spacing;
                let available_width = Self::visible_available_width(ui);
                let gap = 12.0;
                let use_two_columns = node.children.len() > 7 && available_width >= 400.0;
                if use_two_columns {
                    let column_width = ((available_width - gap) * 0.5).max(0.0);
                    for row in node.children.chunks(2) {
                        ui.horizontal_top(|ui| {
                            let mut spacing = ui.spacing().item_spacing;
                            spacing.x = 0.0;
                            ui.spacing_mut().item_spacing = spacing;
                            for (index, child) in row.iter().enumerate() {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(column_width, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(column_width);
                                        ui.set_max_width(column_width);
                                        self.render_form_row(
                                            ui,
                                            child,
                                            state,
                                            label_width,
                                            min_field_width,
                                        );
                                    },
                                );
                                if index + 1 < row.len() {
                                    ui.add_space(gap);
                                }
                            }
                        });
                    }
                } else {
                    for child in &node.children {
                        self.render_form_row(ui, child, state, label_width, min_field_width);
                    }
                }
                ui.spacing_mut().item_spacing = previous_spacing;
            }
            GuiLayoutMode::KeyValueGrid { min_pair_width } => {
                let plan = Self::plan_responsive_columns(
                    Self::visible_available_width(ui),
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
                                    ui.set_max_width(child_width);
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
                let available_width = Self::visible_available_width(ui);
                let buttons_per_row = ((available_width + 12.0) / (min_button_width + 12.0))
                    .floor()
                    .max(1.0) as usize;
                let row_count = node.children.len().div_ceil(buttons_per_row);
                for (row_index, chunk) in node.children.chunks(buttons_per_row).enumerate() {
                    let row_button_width = ((available_width
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
                                    ui.set_max_width(row_button_width);
                                    self.render_button_like(ui, child, state);
                                },
                            );
                        }
                    });
                    if row_index + 1 < row_count {
                        ui.add_space(8.0);
                    }
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
                let buttons_per_row = ((Self::visible_available_width(ui) + gap)
                    / (button_width + gap))
                    .floor()
                    .max(1.0) as usize;
                let row_count = node.children.len().div_ceil(buttons_per_row);
                for (row_index, chunk) in node.children.chunks(buttons_per_row).enumerate() {
                    ui.horizontal_top(|ui| {
                        let available_width = Self::visible_available_width(ui);
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
        state: &SorotteGuiShellAppState,
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
                self.render_status_pair(ui, node);
            }
            GuiWidgetKind::Panel | GuiWidgetKind::List => {}
        }
    }

    fn render_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        tab_width: f32,
    ) {
        let label = if tab_width < 104.0 {
            egui::RichText::new(Self::display_text(node)).small()
        } else {
            egui::RichText::new(Self::display_text(node))
        };
        let tab_height = ui.spacing().interact_size.y;
        let response = ui
            .push_id(&node.id, |ui| {
                ui.add_enabled(
                    node.enabled,
                    egui::Button::new(label)
                        .selected(node.selected)
                        .min_size(egui::vec2(tab_width.max(0.0), tab_height)),
                )
            })
            .inner;
        Self::register_automation_id(ui, &response, node);
        if response.clicked() {
            self.actions
                .extend(Self::actions_for_clicked_button(state, node));
        }
    }
}
