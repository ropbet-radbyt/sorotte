use eframe::egui;

use super::render_io::GuiDroppedFilesRequest;
use super::shell_state::{GuiShellAction, GuiShellModal, SyncplayGuiShellAppState};
use super::widget_tree::{GuiLayoutMode, GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer};

#[cfg(test)]
#[path = "app_render_egui/tests.rs"]
mod tests;

#[derive(Debug, Default)]
pub(super) struct GuiWidgetEguiRenderer {
    stack: Vec<GuiWidgetNode>,
    root: Option<GuiWidgetNode>,
    actions: Vec<GuiShellAction>,
    close_requested: bool,
    playback_prompt_requested: Option<GuiPlaybackPromptKind>,
    selected_media_files: Option<Vec<String>>,
    dropped_files_request: Option<GuiDroppedFilesRequest>,
    playlist_drop_target_rect: Option<egui::Rect>,
    playlist_drop_target_hovered: bool,
    pending_completion_requested: bool,
    pending_cancel_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPlaybackPromptKind {
    Seek,
    Offset,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiResponsiveColumnsPlan {
    pub(super) column_count: usize,
    pub(super) row_count: usize,
    pub(super) column_width: f32,
    pub(super) rows: Vec<Vec<GuiResponsiveColumnsPlanEntry>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GuiResponsiveColumnsPlanEntry {
    pub(super) child_index: usize,
    pub(super) column: usize,
    pub(super) span: usize,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn root(&self) -> Option<&GuiWidgetNode> {
        self.root.as_ref()
    }

    pub(super) fn take_close_requested(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }

    pub(super) fn take_playback_prompt_requested(&mut self) -> Option<GuiPlaybackPromptKind> {
        self.playback_prompt_requested.take()
    }

    pub(super) fn take_selected_media_files(&mut self) -> Option<Vec<String>> {
        self.selected_media_files.take()
    }

    pub(super) fn take_dropped_files_request(&mut self) -> Option<GuiDroppedFilesRequest> {
        self.dropped_files_request.take()
    }

    pub(super) fn take_pending_completion_requested(&mut self) -> bool {
        std::mem::take(&mut self.pending_completion_requested)
    }

    pub(super) fn take_pending_cancel_requested(&mut self) -> bool {
        std::mem::take(&mut self.pending_cancel_requested)
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        state: &SyncplayGuiShellAppState,
        show_manual_pending_controls: bool,
    ) -> Vec<GuiShellAction> {
        self.playlist_drop_target_rect = None;
        self.playlist_drop_target_hovered = false;
        self.dropped_files_request = None;
        if let Some(root) = self.root().cloned() {
            self.show_menu_bar(ctx, &root, state);
            self.show_modal_window(ctx, state);
            self.show_status_bar(ctx, &root, show_manual_pending_controls);
            self.show_navigation_panel(ctx, &root, state);
            self.show_active_surface(ctx, &root, state);
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Syncplay GUI");
                ui.label("No widget tree is currently available.");
            });
        }
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        self.dropped_files_request = Self::dropped_files_request_for_input(
            state,
            self.playlist_drop_target_hovered,
            self.playlist_drop_target_rect,
            ctx.input(|input| input.pointer.hover_pos()),
            dropped_files,
        );
        std::mem::take(&mut self.actions)
    }

    fn show_menu_bar(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(menus) = root.find("menus-root") else {
            return;
        };
        egui::TopBottomPanel::top("syncplay-native-menu-bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for section in &menus.children {
                    ui.menu_button(&section.label, |ui| {
                        self.render_menu_section(ui, section, state);
                    });
                }
            });
        });
    }

    fn show_modal_window(&mut self, ctx: &egui::Context, state: &SyncplayGuiShellAppState) {
        let Some(modal) = state.open_modal else {
            return;
        };
        let mut open = true;
        let mut close_clicked = false;
        egui::Window::new(Self::modal_window_title(modal))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                for line in Self::modal_body_lines(modal, state) {
                    ui.label(line);
                }
                if modal == GuiShellModal::UpdateNotice
                    && let Some(url) = state.update_check.url.as_deref()
                {
                    ui.hyperlink_to("Open update page", url);
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for (_, label, action) in Self::modal_actions(modal) {
                        if ui.button(label).clicked() {
                            self.actions.push(action);
                        }
                    }
                });
                ui.separator();
                if ui.button("Close").clicked() {
                    close_clicked = true;
                }
            });
        if !open || close_clicked {
            self.actions.push(GuiShellAction::CloseModal);
        }
    }

    fn show_status_bar(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        show_manual_pending_controls: bool,
    ) {
        let active_view = root
            .find("shell:active-view")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        let open_modal = root
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        let pending_operation = root
            .find("shell:pending-operation")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(none)");
        let media_index_active = root
            .find("shell:media-index-active")
            .and_then(|node| node.value.as_deref())
            .is_some_and(|value| matches!(value, "yes" | "true"));
        let media_index_status = root
            .find("shell:media-index-status")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(idle)");
        egui::TopBottomPanel::bottom("syncplay-native-status-bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Syncplay GUI");
                ui.separator();
                ui.label(format!("view: {active_view}"));
                ui.separator();
                ui.label(format!("modal: {open_modal}"));
                ui.separator();
                ui.label(format!("pending: {pending_operation}"));
                if media_index_active {
                    ui.separator();
                    ui.add(egui::Spinner::new());
                    ui.label(media_index_status);
                }
                if Self::should_show_manual_pending_controls(
                    pending_operation,
                    show_manual_pending_controls,
                ) {
                    ui.separator();
                    if ui.button("Complete").clicked() {
                        self.pending_completion_requested = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_cancel_requested = true;
                    }
                }
            });
        });
    }

    pub(super) fn should_show_manual_pending_controls(
        pending_operation: &str,
        show_manual_pending_controls: bool,
    ) -> bool {
        show_manual_pending_controls && pending_operation != "(none)"
    }

    fn show_navigation_panel(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        egui::SidePanel::left("syncplay-native-navigation")
            .default_width(200.0)
            .min_width(180.0)
            .max_width(280.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Surfaces");
                ui.separator();
                for child in &root.children {
                    if Self::is_surface_node(child) {
                        let response =
                            ui.add(egui::Button::new(&child.label).selected(child.selected));
                        if response.clicked()
                            && let Some(action) = Self::action_for_surface_node(child)
                        {
                            self.actions.push(action);
                        }
                    }
                }
                if let Some(quick_actions) = root.find("shell:quick-actions") {
                    ui.separator();
                    ui.heading("Quick Actions");
                    for action in &quick_actions.children {
                        self.render_leaf(ui, action, state);
                    }
                }
                Self::render_sidebar_list_branch(ui, root.find("shell:commands"), "Commands");
                Self::render_sidebar_list_branch(ui, root.find("shell:validation"), "Validation");
                if let Some(notifications) = root.find("shell:notifications") {
                    ui.separator();
                    ui.heading("Notifications");
                    if notifications.children.is_empty() {
                        ui.label("No transient notifications.");
                    } else {
                        for notification in &notifications.children {
                            if ui
                                .selectable_label(false, Self::display_text(notification))
                                .clicked()
                                && let Some(action) = Self::action_for_list_item_node(notification)
                            {
                                self.actions.push(action);
                            }
                        }
                    }
                }
            });
    }

    fn render_sidebar_list_branch(
        ui: &mut egui::Ui,
        branch: Option<&GuiWidgetNode>,
        heading: &str,
    ) {
        let Some(branch) = branch else {
            return;
        };
        ui.separator();
        ui.heading(heading);
        if branch.children.is_empty() {
            ui.label("No items.");
        } else {
            for item in &branch.children {
                ui.label(Self::display_text(item));
            }
        }
    }

    fn show_active_surface(
        &mut self,
        ctx: &egui::Context,
        root: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let active_surface = root
            .children
            .iter()
            .find(|node| Self::is_surface_node(node) && node.selected)
            .or_else(|| {
                root.children
                    .iter()
                    .find(|node| Self::is_surface_node(node))
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(active_surface) = active_surface {
                    ui.heading(&active_surface.label);
                    ui.separator();
                    self.render_node(ui, active_surface, state);
                } else {
                    ui.heading(&root.label);
                    ui.label("No active surface is currently selected.");
                }
            });
        });
    }

    fn render_menu_section(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        for child in &node.children {
            if child.children.is_empty() {
                self.render_leaf(ui, child, state);
            } else {
                ui.menu_button(&child.label, |ui| {
                    self.render_menu_section(ui, child, state);
                });
            }
        }
    }

    fn render_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::Layout => self.render_layout(ui, node, state),
            GuiWidgetKind::Panel => {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    if let Some(min_content_height) = node.min_content_height {
                        ui.set_min_height(min_content_height);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&node.label);
                        if node.selected {
                            ui.label(egui::RichText::new("active").small().strong());
                        }
                        if !node.enabled {
                            ui.label(egui::RichText::new("disabled").small());
                        }
                    });
                    for child in &node.children {
                        self.render_node(ui, child, state);
                    }
                });
            }
            GuiWidgetKind::List => {
                let response = egui::Frame::group(ui.style()).show(ui, |ui| {
                    if let Some(min_content_height) = node.min_content_height {
                        ui.set_min_height(min_content_height);
                    }
                    ui.strong(&node.label);
                    if node.children.is_empty() {
                        ui.label("No items.");
                    } else {
                        for child in &node.children {
                            self.render_node(ui, child, state);
                        }
                    }
                });
                if node.id == "main-window:playlist" {
                    self.playlist_drop_target_rect = Some(response.response.rect);
                    self.playlist_drop_target_hovered = response.response.hovered();
                }
            }
            _ => self.render_leaf(ui, node, state),
        }
    }

    fn render_layout(
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
                ui.horizontal_wrapped(|ui| {
                    let mut spacing = ui.spacing().item_spacing;
                    spacing.x = 8.0;
                    spacing.y = 8.0;
                    ui.spacing_mut().item_spacing = spacing;
                    for child in &node.children {
                        self.render_tab_button(ui, child, state, min_tab_width);
                    }
                });
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
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [label_width, 0.0],
                                egui::Label::new(egui::RichText::new(&child.label).strong()),
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
        }
    }

    fn render_leaf(
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
            GuiWidgetKind::Button => self.render_button_like(ui, node, state),
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
                        ui.label(egui::RichText::new(&node.label).strong());
                        ui.label(node.value.as_deref().unwrap_or("(none)"));
                    });
                }
            }
            GuiWidgetKind::Panel | GuiWidgetKind::List => {}
        }
    }

    fn render_text_input(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_else(|| "(none)".to_owned());
        ui.horizontal(|ui| {
            ui.label(&node.label);
            let response = ui.add_enabled(
                node.enabled,
                egui::TextEdit::singleline(&mut value)
                    .password(matches!(node.kind, GuiWidgetKind::PasswordInput))
                    .desired_width(ui.available_width().max(120.0)),
            );
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
        });
    }

    fn render_text_area(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_default();
        ui.vertical(|ui| {
            ui.label(&node.label);
            let response = ui.add_enabled(
                node.enabled,
                egui::TextEdit::multiline(&mut value)
                    .desired_width(ui.available_width().max(160.0))
                    .desired_rows(6),
            );
            if let Some(actions) =
                Self::actions_for_text_input_node(state, node, &value, response.changed(), false)
            {
                self.actions.extend(actions);
            }
        });
    }

    fn render_select(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = node.value.clone().unwrap_or_default();
        let previous = value.clone();
        let options = Self::configuration_select_options_for_node(state, node)
            .unwrap_or_else(|| vec![previous.clone()]);
        ui.horizontal(|ui| {
            ui.label(&node.label);
            ui.add_enabled_ui(node.enabled, |ui| {
                egui::ComboBox::from_id_salt(&node.id)
                    .selected_text(if value.is_empty() { "(unset)" } else { &value })
                    .width(ui.available_width().max(120.0))
                    .show_ui(ui, |ui| {
                        for option in &options {
                            ui.selectable_value(&mut value, option.clone(), option);
                        }
                    });
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
        let available_width = available_width.max(min_column_width);
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

    fn render_key_value_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
            GuiWidgetKind::Status | GuiWidgetKind::ReadOnly => {
                if Self::should_render_combined_status_label(node) {
                    ui.label(Self::display_text(node));
                } else {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&node.label).strong());
                        ui.label(node.value.as_deref().unwrap_or("(none)"));
                    });
                }
            }
            _ => self.render_node(ui, node, state),
        }
    }

    fn render_field_control(
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
                let mut value = node.value.clone().unwrap_or_else(|| "(none)".to_owned());
                if !omit_label {
                    ui.label(&node.label);
                }
                let response = ui.add_enabled(
                    node.enabled,
                    egui::TextEdit::singleline(&mut value)
                        .password(matches!(node.kind, GuiWidgetKind::PasswordInput))
                        .desired_width(ui.available_width().max(120.0)),
                );
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
                        .desired_width(ui.available_width().max(160.0))
                        .desired_rows(6),
                );
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
                let mut value = node.value.clone().unwrap_or_default();
                let previous = value.clone();
                let options = Self::configuration_select_options_for_node(state, node)
                    .unwrap_or_else(|| vec![previous.clone()]);
                if !omit_label {
                    ui.label(&node.label);
                }
                ui.add_enabled_ui(node.enabled, |ui| {
                    egui::ComboBox::from_id_salt(&node.id)
                        .selected_text(if value.is_empty() { "(unset)" } else { &value })
                        .width(ui.available_width().max(120.0))
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
            GuiWidgetKind::Button => self.render_button_like(ui, node, state),
            GuiWidgetKind::ReadOnly | GuiWidgetKind::Status => {
                if omit_label {
                    ui.label(node.value.as_deref().unwrap_or("(none)"));
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

    fn render_button_like(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut clicked = false;
        ui.add_enabled_ui(node.enabled, |ui| {
            clicked = ui
                .add_sized(
                    [ui.available_width().max(0.0), 0.0],
                    egui::Button::new(Self::display_text(node)),
                )
                .clicked();
        });
        if clicked {
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
    }

    fn render_tab_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        min_tab_width: f32,
    ) {
        let mut clicked = false;
        ui.add_enabled_ui(node.enabled, |ui| {
            clicked = ui
                .add_sized(
                    [min_tab_width.max(0.0), 0.0],
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

    pub(super) fn modal_window_title(modal: GuiShellModal) -> &'static str {
        match modal {
            GuiShellModal::TlsCertificatePrompt => "TLS Certificate Prompt",
            GuiShellModal::UpdateNotice => "Update Notice",
            GuiShellModal::About => "About Syncplay",
        }
    }

    fn modal_body_lines(modal: GuiShellModal, state: &SyncplayGuiShellAppState) -> Vec<String> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                "A TLS certificate prompt is active for the current connection.".to_owned(),
                "Trust the certificate for this session or reject it to keep the warning visible."
                    .to_owned(),
            ],
            GuiShellModal::UpdateNotice => state
                .update_check
                .body_lines(Some(state.runtime_language_tag_legacy_compatible())),
            GuiShellModal::About => vec![
                "The reducer reports that the About dialog is open.".to_owned(),
                "This modal now routes into the existing help and update actions.".to_owned(),
            ],
        }
    }

    pub(super) fn modal_actions(
        modal: GuiShellModal,
    ) -> Vec<(&'static str, &'static str, GuiShellAction)> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                (
                    "shell:modal:tls:trust",
                    "Trust Certificate",
                    GuiShellAction::TrustTlsCertificatePrompt,
                ),
                (
                    "shell:modal:tls:reject",
                    "Reject Certificate",
                    GuiShellAction::RejectTlsCertificatePrompt,
                ),
                (
                    "shell:modal:tls:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
            ],
            GuiShellModal::UpdateNotice => vec![
                (
                    "shell:modal:update:dismiss",
                    "Dismiss Notice",
                    GuiShellAction::DismissUpdateNotice,
                ),
                (
                    "shell:modal:update:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
                (
                    "shell:modal:update:check-again",
                    "Check Again",
                    GuiShellAction::AnnounceUpdateNoticeAvailable,
                ),
            ],
            GuiShellModal::About => vec![
                (
                    "shell:modal:about:help",
                    "Open Help",
                    GuiShellAction::AnnounceHelpRequested,
                ),
                (
                    "shell:modal:about:update",
                    "Check for Updates",
                    GuiShellAction::AnnounceUpdateNoticeAvailable,
                ),
            ],
        }
    }

    fn display_text(node: &GuiWidgetNode) -> String {
        match node.value.as_deref() {
            Some(value) if !value.is_empty() => format!("{}: {}", node.label, value),
            _ => node.label.clone(),
        }
    }

    fn should_render_combined_status_label(node: &GuiWidgetNode) -> bool {
        node.id.starts_with("media-search:timing:")
            || node.id.starts_with("shell:command:")
            || node.id.starts_with("shell:validation:")
    }

    fn is_surface_node(node: &GuiWidgetNode) -> bool {
        matches!(
            node.id.as_str(),
            "configuration-root"
                | "main-window-root"
                | "public-servers-root"
                | "media-search-root"
                | "menus-root"
        )
    }
}

impl GuiWidgetRenderer for GuiWidgetEguiRenderer {
    fn begin_node(&mut self, node: &GuiWidgetNode, _depth: usize) {
        let mut shallow_node = node.clone();
        shallow_node.children.clear();
        self.stack.push(shallow_node);
    }

    fn end_node(&mut self, _node: &GuiWidgetNode, _depth: usize) {
        let Some(completed_node) = self.stack.pop() else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(completed_node);
        } else {
            self.root = Some(completed_node);
        }
    }
}
