use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use super::{GuiResponsiveColumnsPlan, GuiResponsiveColumnsPlanEntry, GuiWidgetEguiRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiCompactActionIcon {
    AddFile,
    AddUrl,
    PlaylistOptions,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn editable_text_value(node: &GuiWidgetNode) -> String {
        node.value.clone().unwrap_or_default()
    }

    pub(super) fn render_setup_command_bar(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        for child in &node.children {
            self.render_node(ui, child, state);
        }
    }

    pub(super) fn render_text_input(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        self.render_form_row(ui, node, state, 140.0, 180.0);
    }

    pub(super) fn render_text_area(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        self.render_form_row(ui, node, state, 140.0, 180.0);
    }

    pub(super) fn render_select(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        self.render_form_row(ui, node, state, 140.0, 180.0);
    }

    pub(super) fn render_form_row(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        label_width: f32,
        min_field_width: f32,
    ) {
        let available_width = Self::visible_available_width(ui);
        let gap = 10.0;
        let label_width = Self::form_label_width(available_width, label_width);
        let field_width = (available_width - label_width - gap).max(0.0);
        let stacked = field_width < min_field_width || available_width < 280.0;

        if stacked {
            self.render_form_label(ui, node, available_width, false);
            ui.add_space(3.0);
            ui.allocate_ui_with_layout(
                egui::vec2(available_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(available_width);
                    ui.set_max_width(available_width);
                    self.render_field_control(ui, node, state, true);
                },
            );
            ui.add_space(4.0);
            return;
        }

        ui.horizontal_top(|ui| {
            let label_align_top = matches!(node.kind, GuiWidgetKind::TextArea);
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(if label_align_top {
                    egui::Align::Min
                } else {
                    egui::Align::Center
                }),
                |ui| {
                    ui.set_width(label_width);
                    ui.set_max_width(label_width);
                    self.render_form_label(ui, node, label_width, true);
                },
            );
            ui.add_space(gap);
            ui.allocate_ui_with_layout(
                egui::vec2(field_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(field_width);
                    ui.set_max_width(field_width);
                    self.render_field_control(ui, node, state, true);
                },
            );
        });
    }

    pub(super) fn form_label_width(available_width: f32, preferred_width: f32) -> f32 {
        preferred_width
            .min((available_width * 0.38).max(96.0))
            .min(available_width.max(0.0))
            .max(0.0)
    }

    fn render_form_label(
        &self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        width: f32,
        truncate: bool,
    ) {
        let label = egui::Label::new(
            egui::RichText::new(&node.label)
                .strong()
                .color(Self::palette_for_ui(ui).muted_text),
        );
        let label = if truncate { label.truncate() } else { label };
        ui.set_width(width.max(0.0));
        ui.set_max_width(width.max(0.0));
        let response = ui.add(label);
        if truncate {
            response.on_hover_text(node.label.clone());
        }
    }

    pub(super) fn render_status_pair(&mut self, ui: &mut egui::Ui, node: &GuiWidgetNode) {
        if Self::should_render_combined_status_label(node) {
            ui.label(Self::display_text(node));
            return;
        }

        let available_width = Self::visible_available_width(ui);
        let gap = 8.0;
        let label_width = Self::form_label_width(available_width, 150.0);
        let value_width = (available_width - label_width - gap).max(0.0);
        let stacked = value_width < 100.0 || available_width < 300.0;
        if stacked {
            self.render_form_label(ui, node, available_width, false);
            ui.label(Self::display_status_rich_text(ui, node));
            return;
        }

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(label_width);
                    ui.set_max_width(label_width);
                    self.render_form_label(ui, node, label_width, true);
                },
            );
            ui.add_space(gap);
            ui.allocate_ui_with_layout(
                egui::vec2(value_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(value_width);
                    ui.set_max_width(value_width);
                    let value = Self::display_status_value(node);
                    let response = ui
                        .add(egui::Label::new(Self::display_status_rich_text(ui, node)).truncate());
                    response.on_hover_text(value);
                },
            );
        });
    }

    fn render_raw_select(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_default();
        let previous = value.clone();
        let options = Self::configuration_select_options_for_node(state, node)
            .unwrap_or_else(|| vec![previous.clone()]);
        ui.add_enabled_ui(node.enabled, |ui| {
            egui::ComboBox::from_id_salt(&node.id)
                .selected_text(if value.is_empty() { "(unset)" } else { &value })
                .width(Self::visible_available_width(ui).max(1.0))
                .show_ui(ui, |ui| {
                    for option in &options {
                        ui.selectable_value(&mut value, option.clone(), option);
                    }
                });
        });
        if value != previous
            && let Some(actions) =
                Self::actions_for_text_input_node(state, node, &value, true, false)
        {
            self.actions.extend(actions);
        }
    }

    pub(super) fn plan_responsive_columns<I>(
        available_width: f32,
        gap: f32,
        min_column_width: f32,
        max_columns: usize,
        spans: I,
    ) -> GuiResponsiveColumnsPlan
    where
        I: IntoIterator<Item = usize>,
    {
        let max_columns = max_columns.max(1);
        let min_column_width = min_column_width.max(1.0);
        let available_width = available_width.max(0.0);
        let column_count = (((available_width + gap) / (min_column_width + gap)).floor() as usize)
            .clamp(1, max_columns);
        let column_width = ((available_width - (gap * (column_count.saturating_sub(1)) as f32))
            / column_count as f32)
            .max(0.0);
        let mut rows: Vec<Vec<GuiResponsiveColumnsPlanEntry>> = Vec::new();
        let mut current_row: Vec<GuiResponsiveColumnsPlanEntry> = Vec::new();
        let mut current_column = 0usize;

        for (child_index, requested_span) in spans.into_iter().enumerate() {
            let span = requested_span.max(1).min(column_count);
            if !current_row.is_empty() && (current_column + span > column_count) {
                rows.push(current_row);
                current_row = Vec::new();
                current_column = 0;
            }
            current_row.push(GuiResponsiveColumnsPlanEntry {
                child_index,
                column: current_column,
                span,
            });
            current_column += span;
            if current_column >= column_count {
                rows.push(current_row);
                current_row = Vec::new();
                current_column = 0;
            }
        }

        if !current_row.is_empty() {
            rows.push(current_row);
        }

        GuiResponsiveColumnsPlan {
            column_count,
            row_count: rows.len(),
            column_width,
            rows,
        }
    }

    pub(super) fn render_key_value_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::Status | GuiWidgetKind::ReadOnly => {
                self.render_status_pair(ui, node);
            }
            _ => self.render_node(ui, node, state),
        }
    }

    pub(super) fn render_field_control(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        omit_label: bool,
    ) {
        match node.kind {
            GuiWidgetKind::TextInput
            | GuiWidgetKind::PasswordInput
            | GuiWidgetKind::NumericInput => {
                let mut value = Self::editable_text_value(node);
                if !omit_label {
                    ui.label(&node.label);
                }
                let response = ui.add_enabled(
                    node.enabled,
                    egui::TextEdit::singleline(&mut value)
                        .password(matches!(node.kind, GuiWidgetKind::PasswordInput))
                        .desired_width(Self::visible_available_width(ui).max(1.0)),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        response.enabled(),
                        node.label.clone(),
                    )
                });
                let submitted =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if let Some(actions) = Self::actions_for_text_input_node(
                    state,
                    node,
                    &value,
                    response.changed(),
                    submitted,
                ) {
                    self.actions.extend(actions);
                }
            }
            GuiWidgetKind::TextArea => {
                let mut value = node.value.clone().unwrap_or_default();
                if !omit_label {
                    ui.label(&node.label);
                }
                let response = ui.add_enabled(
                    node.enabled,
                    egui::TextEdit::multiline(&mut value)
                        .desired_width(Self::visible_available_width(ui).max(1.0))
                        .desired_rows(Self::text_area_rows_for_node(node)),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        response.enabled(),
                        node.label.clone(),
                    )
                });
                if let Some(actions) = Self::actions_for_text_input_node(
                    state,
                    node,
                    &value,
                    response.changed(),
                    false,
                ) {
                    self.actions.extend(actions);
                }
            }
            GuiWidgetKind::Select => {
                if !omit_label {
                    ui.label(&node.label);
                }
                self.render_raw_select(ui, node, state);
            }
            GuiWidgetKind::Checkbox => {
                let mut checked = matches!(node.value.as_deref(), Some("yes" | "true"));
                let checkbox_label = if omit_label { "" } else { &node.label };
                let response = ui.add_enabled(
                    node.enabled,
                    egui::Checkbox::new(&mut checked, checkbox_label),
                );
                if response.changed()
                    && let Some(action) = Self::action_for_checkbox_node(state, node, checked)
                {
                    self.actions.push(action);
                }
            }
            GuiWidgetKind::Button => {
                self.render_button_like(ui, node, state);
            }
            GuiWidgetKind::ReadOnly | GuiWidgetKind::Status => {
                if omit_label {
                    ui.label(Self::display_status_rich_text(ui, node));
                } else {
                    self.render_leaf(ui, node, state);
                }
            }
            GuiWidgetKind::Layout
            | GuiWidgetKind::Panel
            | GuiWidgetKind::List
            | GuiWidgetKind::ListItem => {
                self.render_node(ui, node, state);
            }
        }
    }

    fn text_area_rows_for_node(node: &GuiWidgetNode) -> usize {
        if matches!(
            node.id.as_str(),
            "main-window:playlist-url-edit:text" | "main-window:media-url-edit:text"
        ) {
            3
        } else if node.id == "config:Connection:Room History" {
            1
        } else {
            6
        }
    }

    pub(super) fn render_button_like(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) -> bool {
        if !node.children.is_empty() {
            ui.add_enabled_ui(node.enabled, |ui| {
                ui.menu_button(Self::display_text(node), |ui| {
                    self.render_menu_section(ui, node, state);
                });
            });
            return false;
        }
        if Self::playback_control_icon(node).is_some() {
            return self.render_playback_icon_button(ui, node, state);
        }
        if node.id == "main-window:control:set-ready" {
            return self.render_playback_ready_button(ui, node, state);
        }
        if node.id.ends_with(":close") {
            return self.render_panel_close_button(ui, node, state);
        }
        let mut label = egui::RichText::new(Self::display_text(node));
        if node.enabled
            && let Some((_, _, text_color)) = Self::button_colors_for_node(ui, node)
        {
            label = label.color(text_color).strong();
        }
        let mut button =
            egui::Button::new(label).min_size(egui::vec2(Self::visible_available_width(ui), 0.0));
        if node.enabled
            && let Some((fill, hover_fill, _)) = Self::button_colors_for_node(ui, node)
        {
            button = button.fill(
                if ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
                    hover_fill
                } else {
                    fill
                },
            );
        }
        let response = ui.add_enabled(node.enabled, button);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                response.enabled(),
                Self::display_text(node),
            )
        });
        let response = Self::attach_node_tooltip(response, node);
        let clicked = response.clicked();
        if clicked {
            self.handle_button_node_click(state, node);
        }
        clicked
    }

    const COMPACT_ACTION_BUTTON_HEIGHT: f32 = 32.0;
    const COMPACT_ACTION_BUTTON_MIN_WIDTH: f32 = 86.0;
    const COMPACT_ACTION_BUTTON_MAX_WIDTH: f32 = 136.0;
    pub(super) const COMPACT_ACTION_BUTTON_GAP: f32 = 8.0;

    pub(super) fn compact_action_button_size(node: &GuiWidgetNode) -> egui::Vec2 {
        if Self::compact_action_icon(node).is_some() {
            return egui::vec2(if node.children.is_empty() { 40.0 } else { 52.0 }, 40.0);
        }
        let text_width = Self::display_text(node).chars().count() as f32 * 7.5;
        let menu_indicator_width = if node.children.is_empty() { 0.0 } else { 14.0 };
        let horizontal_padding = 28.0;
        egui::vec2(
            (text_width + menu_indicator_width + horizontal_padding).clamp(
                Self::COMPACT_ACTION_BUTTON_MIN_WIDTH,
                Self::COMPACT_ACTION_BUTTON_MAX_WIDTH,
            ),
            Self::COMPACT_ACTION_BUTTON_HEIGHT,
        )
    }

    pub(super) fn render_compact_action_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let desired_size = Self::compact_action_button_size(node);
        let response = ui
            .push_id(&node.id, |ui| {
                if node.children.is_empty() {
                    ui.add_enabled(
                        node.enabled,
                        egui::Button::new("")
                            .frame(false)
                            .min_size(desired_size)
                            .sense(egui::Sense::click()),
                    )
                } else {
                    ui.add_enabled_ui(node.enabled, |ui| {
                        let menu_button = egui::Button::new("")
                            .frame(false)
                            .min_size(desired_size)
                            .sense(egui::Sense::click());
                        let (response, _) = egui::containers::menu::MenuButton::from_button(
                            menu_button,
                        )
                        .ui(ui, |ui| {
                            self.render_menu_section(ui, node, state);
                        });
                        response
                    })
                    .inner
                }
            })
            .inner;

        let label = Self::display_text(node).to_owned();
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label.clone())
        });
        let response = Self::attach_hover_text(response, label.clone());
        Self::paint_compact_action_button(ui, &response, node, &label);
        if node.children.is_empty() && response.clicked() {
            self.handle_button_node_click(state, node);
        }
    }

    fn compact_action_icon(node: &GuiWidgetNode) -> Option<GuiCompactActionIcon> {
        match node.id.as_str() {
            "main-window:playlist:add-files" => Some(GuiCompactActionIcon::AddFile),
            "main-window:playlist:add-url" => Some(GuiCompactActionIcon::AddUrl),
            "main-window:playlist:more-menu" => Some(GuiCompactActionIcon::PlaylistOptions),
            _ => None,
        }
    }

    fn paint_compact_action_button(
        ui: &egui::Ui,
        response: &egui::Response,
        node: &GuiWidgetNode,
        label: &str,
    ) {
        let palette = Self::palette_for_ui(ui);
        let visuals = ui.visuals();
        let enabled = response.enabled() && node.enabled;
        let pressed = response.is_pointer_button_down_on();
        let hovered = response.hovered();
        let fill = if !enabled {
            visuals.widgets.inactive.bg_fill.gamma_multiply(0.55)
        } else if pressed {
            palette.info_bg.gamma_multiply(0.92)
        } else if hovered {
            palette.info_bg.gamma_multiply(0.78)
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let stroke_color = if !enabled {
            visuals
                .widgets
                .inactive
                .bg_stroke
                .color
                .gamma_multiply(0.65)
        } else if hovered || pressed {
            palette.info_border
        } else {
            palette.neutral_border
        };
        let text_color = if !enabled {
            visuals.weak_text_color()
        } else if hovered || pressed {
            palette.info_text
        } else {
            palette.neutral_text
        };
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            5,
            fill,
            egui::Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Inside,
        );

        if let Some(icon) = Self::compact_action_icon(node) {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(
                    if node.children.is_empty() {
                        rect.center().x
                    } else {
                        rect.left() + 18.0
                    },
                    rect.center().y,
                ),
                egui::vec2(21.0, 21.0),
            );
            Self::paint_compact_action_icon(ui, icon_rect, icon, text_color);
            if !node.children.is_empty() {
                Self::paint_compact_menu_indicator(ui, rect, text_color);
            }
            return;
        }

        let text_left = rect.left() + 12.0;
        let indicator_width = if node.children.is_empty() { 0.0 } else { 14.0 };
        let text_right = (rect.right() - 10.0 - indicator_width).max(text_left);
        let text_width = (text_right - text_left).max(0.0);
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let (display_label, _truncated) = Self::truncate_single_line_text_for_width(
            ui,
            label,
            font_id.clone(),
            text_color,
            text_width,
        );
        let galley = ui
            .painter()
            .layout_no_wrap(display_label, font_id, text_color);
        ui.painter().with_clip_rect(rect).galley(
            egui::pos2(text_left, rect.center().y - (galley.size().y * 0.5)),
            galley,
            text_color,
        );

        if !node.children.is_empty() {
            Self::paint_compact_menu_indicator(ui, rect, text_color);
        }
    }

    fn paint_compact_action_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        icon: GuiCompactActionIcon,
        color: egui::Color32,
    ) {
        let stroke = egui::Stroke::new(1.8, color);
        match icon {
            GuiCompactActionIcon::AddFile => {
                let body = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 2.0, rect.top() + 1.0),
                    egui::pos2(rect.right() - 5.0, rect.bottom() - 1.0),
                );
                ui.painter()
                    .rect_stroke(body, 1, stroke, egui::StrokeKind::Inside);
                ui.painter().line_segment(
                    [
                        egui::pos2(body.right() - 5.0, body.top()),
                        egui::pos2(body.right(), body.top() + 5.0),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(body.right(), body.top() + 5.0),
                        egui::pos2(body.right(), body.bottom()),
                    ],
                    stroke,
                );
                Self::paint_small_plus(
                    ui,
                    egui::pos2(rect.right() - 3.0, rect.bottom() - 4.0),
                    color,
                );
            }
            GuiCompactActionIcon::AddUrl => {
                let left = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 1.0, rect.center().y - 4.0),
                    egui::vec2(11.0, 8.0),
                );
                let right = left.translate(egui::vec2(9.0, 0.0));
                ui.painter().circle_stroke(left.center(), 5.0, stroke);
                ui.painter().circle_stroke(right.center(), 5.0, stroke);
                ui.painter().line_segment(
                    [
                        egui::pos2(left.center().x + 3.0, left.center().y),
                        egui::pos2(right.center().x - 3.0, right.center().y),
                    ],
                    stroke,
                );
                Self::paint_small_plus(
                    ui,
                    egui::pos2(rect.right() - 2.5, rect.bottom() - 4.0),
                    color,
                );
            }
            GuiCompactActionIcon::PlaylistOptions => {
                for row in 0..3 {
                    let y = rect.top() + 4.0 + (row as f32 * 6.0);
                    ui.painter()
                        .circle_filled(egui::pos2(rect.left() + 4.0, y), 1.5, color);
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left() + 9.0, y),
                            egui::pos2(rect.right() - 2.0, y),
                        ],
                        stroke,
                    );
                }
            }
        }
    }

    fn paint_small_plus(ui: &egui::Ui, center: egui::Pos2, color: egui::Color32) {
        let stroke = egui::Stroke::new(1.8, color);
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 4.0, center.y),
                egui::pos2(center.x + 4.0, center.y),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y - 4.0),
                egui::pos2(center.x, center.y + 4.0),
            ],
            stroke,
        );
    }

    fn paint_compact_menu_indicator(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
        let center = egui::pos2(rect.right() - 12.0, rect.center().y + 1.0);
        let stroke = egui::Stroke::new(1.4, color);
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 4.0, center.y - 2.0),
                egui::pos2(center.x, center.y + 2.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y + 2.0),
                egui::pos2(center.x + 4.0, center.y - 2.0),
            ],
            stroke,
        );
    }

    pub(super) fn button_colors_for_node(
        ui: &egui::Ui,
        node: &GuiWidgetNode,
    ) -> Option<(egui::Color32, egui::Color32, egui::Color32)> {
        let palette = Self::palette_for_ui(ui);
        match node.id.as_str() {
            "config-command:connect"
            | "config-command:save"
            | "configuration:alert:fix-player-path"
            | "main-window:connection:connect"
            | "main-window:room:join"
            | "main-window:room:set"
            | "main-window:room-actions:create-controlled-room"
            | "main-window:room-actions:identify-controller"
            | "main-window:playlist-url-edit:commit"
            | "main-window:media-url-edit:commit"
            | "main-window:chat:send"
            | "plugins:plex:connect"
            | "plugins:plex:poll-auth"
            | "plugins:plex:refresh-servers"
            | "plugins:plex:enable-sync"
            | "plugins:stream-support:install"
            | "plugins:stream-support:alert:install"
            | "plugins:stream-support:recheck"
            | "plugins:stream-support:alert:recheck" => {
                Some((palette.primary, palette.primary_hover, palette.primary_text))
            }
            "config-command:disconnect"
            | "main-window:connection:disconnect"
            | "main-window:room:leave"
            | "main-window:media-url-edit:cancel"
            | "plugins:plex:disconnect" => {
                Some((palette.danger, palette.danger_hover, palette.danger_text))
            }
            _ => None,
        }
    }

    pub(super) fn handle_button_node_click(
        &mut self,
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) {
        if node.id == "shell:quick:open-media-file"
            || Self::is_open_media_file_menu_action(state, node)
        {
            self.selected_media_files = Self::pick_media_files(state);
        } else if Self::is_exit_menu_action(state, node) {
            self.close_requested = true;
        } else if let Some(actions) = Self::direct_menu_actions(state, node) {
            self.actions.extend(actions);
        } else {
            self.actions
                .extend(Self::actions_for_clicked_button(state, node));
        }
    }

    pub(super) fn attach_hover_text(
        response: egui::Response,
        hover_text: impl Into<String>,
    ) -> egui::Response {
        let hover_text = hover_text.into();
        response
            .on_hover_text(hover_text.clone())
            .on_disabled_hover_text(hover_text)
    }

    pub(super) fn attach_node_tooltip(
        response: egui::Response,
        node: &GuiWidgetNode,
    ) -> egui::Response {
        if let Some(tooltip) = node.tooltip.as_ref() {
            response
                .on_hover_text(tooltip.clone())
                .on_disabled_hover_text(tooltip.clone())
        } else {
            response
        }
    }
}
