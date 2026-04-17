use std::time::Duration;

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
    playlist_drop_target_slot: Option<usize>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuiDraggedPlaylistRow {
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiPlaybackControlIcon {
    Play,
    Pause,
    TogglePause,
    Seek,
    UndoSeek,
    SetOffset,
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
        let hovered_files_active = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        let external_file_drag_active = hovered_files_active || !dropped_files.is_empty();
        if !external_file_drag_active {
            self.playlist_drop_target_rect = None;
            self.playlist_drop_target_hovered = false;
            self.playlist_drop_target_slot = None;
        }
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
        self.dropped_files_request = Self::dropped_files_request_for_input(
            state,
            self.playlist_drop_target_hovered,
            self.playlist_drop_target_rect,
            self.playlist_drop_target_slot,
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
                    for (id, label) in Self::modal_actions(modal) {
                        if ui
                            .add_enabled(
                                Self::modal_action_enabled(state, id),
                                egui::Button::new(label),
                            )
                            .clicked()
                        {
                            self.actions
                                .extend(Self::modal_button_actions(state, id, label));
                        }
                    }
                });
                if Self::modal_close_enabled(state, modal) {
                    ui.separator();
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
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
        if node.id == "main-window:browser" {
            self.render_room_browser(ui, node, state);
            return;
        }
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
                if node.id == "main-window:playlist" {
                    self.render_playlist_list(ui, node, state);
                    return;
                }
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

    fn render_playlist_list(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let playlist_focus_id = Self::playlist_focus_id();
        let playlist_focused = ui.ctx().memory(|mem| mem.has_focus(playlist_focus_id));
        let playlist_len = node.children.len();
        let mut first_row_rect = None;
        let mut last_row_rect = None;
        let mut playlist_focus_requested = false;
        let response = egui::Frame::group(ui.style()).show(ui, |ui| {
            if let Some(min_content_height) = node.min_content_height {
                ui.set_min_height(min_content_height);
            }
            ui.strong(&node.label);
            if node.children.is_empty() {
                ui.label("No items.");
            } else {
                for child in &node.children {
                    let row_rect = self.render_playlist_list_item(
                        ui,
                        child,
                        state,
                        playlist_len,
                        playlist_focused,
                        &mut playlist_focus_requested,
                    );
                    if first_row_rect.is_none() {
                        first_row_rect = row_rect;
                    }
                    if row_rect.is_some() {
                        last_row_rect = row_rect;
                    }
                }
            }
            if self.playlist_drop_target_slot.is_none()
                && let Some(pointer_pos) = ui.ctx().pointer_hover_pos()
            {
                if playlist_len == 0
                    || first_row_rect.is_some_and(|rect| pointer_pos.y < rect.top())
                {
                    self.playlist_drop_target_slot = Some(0);
                } else if last_row_rect.is_some_and(|rect| pointer_pos.y > rect.bottom()) {
                    self.playlist_drop_target_slot = Some(playlist_len);
                }
            }
        });
        let playlist_focus_response = ui.interact(
            response.response.rect,
            playlist_focus_id,
            Self::playlist_focus_sense(),
        );
        if playlist_focus_requested {
            playlist_focus_response.request_focus();
        }
        self.playlist_drop_target_rect = Some(response.response.rect);
        self.playlist_drop_target_hovered = response.response.hovered();
    }

    fn render_playlist_list_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
        playlist_len: usize,
        playlist_focused: bool,
        playlist_focus_requested: &mut bool,
    ) -> Option<egui::Rect> {
        let Some(index) = node
            .id
            .strip_prefix("main-window:playlist:")
            .and_then(|suffix| suffix.parse::<usize>().ok())
        else {
            self.render_leaf(ui, node, state);
            return None;
        };
        let can_drag_reorder = node.enabled && state.main_window.playback.can_manage_playlist;
        let text = Self::display_text(node).to_owned();

        let button_response = ui
            .push_id(&node.id, |ui| {
                ui.add_enabled_ui(node.enabled, |ui| {
                    ui.add_sized(
                        [ui.available_width().max(0.0), 0.0],
                        egui::Button::new(text)
                            .selected(node.selected)
                            .sense(Self::playlist_row_sense(can_drag_reorder)),
                    )
                })
                .inner
            })
            .inner;

        if can_drag_reorder {
            if button_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            button_response.dnd_set_drag_payload(GuiDraggedPlaylistRow { index });
        }

        let pointer_actions = Self::playlist_row_pointer_actions(
            index,
            button_response.clicked(),
            button_response.double_clicked(),
        );
        if !pointer_actions.is_empty() {
            button_response.surrender_focus();
            *playlist_focus_requested = true;
            self.actions.extend(pointer_actions);
        }

        self.actions.extend(Self::playlist_row_shortcut_actions(
            state,
            index,
            node.enabled,
            playlist_focused,
            ui.input(|input| input.key_pressed(egui::Key::Enter)),
            ui.input(|input| input.key_pressed(egui::Key::Delete)),
        ));

        self.update_playlist_drop_target_slot(ui, &button_response, index);
        self.render_playlist_drop_indicator(ui, &button_response, index, playlist_len);
        Some(button_response.rect)
    }

    fn render_room_browser(
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
        let frame = egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(10, 10))
            .fill(
                ui.visuals()
                    .widgets
                    .noninteractive
                    .bg_fill
                    .gamma_multiply(0.08),
            );

        frame.show(ui, |ui| {
            if let Some(min_content_height) = node.min_content_height {
                ui.set_min_height(min_content_height);
            }

            ui.horizontal_wrapped(|ui| {
                ui.strong(&node.label);
                Self::render_room_browser_chip(
                    ui,
                    if room_nodes.len() == 1 {
                        "1 room".to_owned()
                    } else {
                        format!("{} rooms", room_nodes.len())
                    },
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .bg_fill
                        .gamma_multiply(0.6),
                    ui.visuals().weak_text_color(),
                );
                Self::render_room_browser_chip(
                    ui,
                    if user_count == 1 {
                        "1 user".to_owned()
                    } else {
                        format!("{user_count} users")
                    },
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .bg_fill
                        .gamma_multiply(0.6),
                    ui.visuals().weak_text_color(),
                );
                if state.main_window.hide_empty_rooms {
                    Self::render_room_browser_chip(
                        ui,
                        "Empty Hidden",
                        ui.visuals().selection.bg_fill.gamma_multiply(0.15),
                        ui.visuals().selection.stroke.color,
                    );
                }
            });
            ui.add_space(8.0);

            if room_nodes.is_empty() {
                let empty_text = empty_node
                    .and_then(|child| child.value.as_deref())
                    .unwrap_or("No visible rooms.");
                ui.label(egui::RichText::new(empty_text).small().weak());
                return;
            }

            self.render_room_browser_room_grid(ui, &room_nodes, state);
        });
    }

    fn render_room_browser_room_grid(
        &mut self,
        ui: &mut egui::Ui,
        room_nodes: &[&GuiWidgetNode],
        state: &SyncplayGuiShellAppState,
    ) {
        let plan = Self::plan_responsive_columns(
            ui.available_width(),
            12.0,
            320.0,
            2,
            room_nodes.iter().map(|_| 1usize),
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
                            self.render_room_browser_room_card(
                                ui,
                                room_nodes[entry.child_index],
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
                ui.add_space(12.0);
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
        let room_fill = if room_node.selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.12)
        } else {
            ui.visuals()
                .widgets
                .noninteractive
                .bg_fill
                .gamma_multiply(0.16)
        };
        let room_stroke = if room_node.selected {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(10, 8))
            .fill(room_fill)
            .stroke(room_stroke)
            .corner_radius(10)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(&room_node.label);
                    if let Some(join_button) = join_button
                        .filter(|button| button.enabled || button.label != "Current Room")
                    {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_room_browser_button(ui, join_button, state)
                        });
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if room_node.selected {
                        Self::render_room_browser_chip(
                            ui,
                            "Current",
                            ui.visuals().selection.bg_fill.gamma_multiply(0.18),
                            ui.visuals().selection.stroke.color,
                        );
                    }
                    if room_state
                        .and_then(|status| status.value.as_deref())
                        .is_some_and(|value| Self::browser_status_flag(value, "controlled"))
                    {
                        Self::render_room_browser_chip(
                            ui,
                            "Controlled",
                            ui.visuals().widgets.active.bg_fill.gamma_multiply(0.18),
                            ui.visuals().widgets.active.fg_stroke.color,
                        );
                    }
                    Self::render_room_browser_chip(
                        ui,
                        if user_nodes.len() == 1 {
                            "1 user".to_owned()
                        } else {
                            format!("{} users", user_nodes.len())
                        },
                        ui.visuals()
                            .widgets
                            .noninteractive
                            .bg_fill
                            .gamma_multiply(0.6),
                        ui.visuals().weak_text_color(),
                    );
                });

                if !user_nodes.is_empty() {
                    ui.add_space(8.0);
                    self.render_room_browser_user_grid(ui, &user_nodes, state);
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

    fn render_room_browser_user_grid(
        &mut self,
        ui: &mut egui::Ui,
        user_nodes: &[&GuiWidgetNode],
        state: &SyncplayGuiShellAppState,
    ) {
        let plan = Self::plan_responsive_columns(
            ui.available_width(),
            10.0,
            250.0,
            2,
            user_nodes.iter().map(|_| 1usize),
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
                            self.render_room_browser_user_card(
                                ui,
                                user_nodes[entry.child_index],
                                state,
                            );
                        },
                    );
                    if entry_index + 1 < row.len() {
                        ui.add_space(10.0);
                    }
                }
            });
            if row_index + 1 < plan.row_count {
                ui.add_space(10.0);
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
        let card_fill = if user_node.selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.10)
        } else {
            ui.visuals()
                .widgets
                .noninteractive
                .bg_fill
                .gamma_multiply(0.10)
        };
        let card_stroke = if user_node.selected {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 7))
            .fill(card_fill)
            .stroke(card_stroke)
            .corner_radius(8)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(&user_node.label).strong());
                    if user_state
                        .and_then(|status| status.value.as_deref())
                        .is_some_and(|value| Self::browser_status_flag(value, "self"))
                    {
                        Self::render_room_browser_chip(
                            ui,
                            "You",
                            ui.visuals().selection.bg_fill.gamma_multiply(0.18),
                            ui.visuals().selection.stroke.color,
                        );
                    }
                    if user_state
                        .and_then(|status| status.value.as_deref())
                        .is_some_and(|value| Self::browser_status_flag(value, "ready"))
                    {
                        Self::render_room_browser_chip(
                            ui,
                            "Ready",
                            ui.visuals().widgets.active.bg_fill.gamma_multiply(0.18),
                            ui.visuals().widgets.active.fg_stroke.color,
                        );
                    }
                    if user_state
                        .and_then(|status| status.value.as_deref())
                        .is_some_and(|value| Self::browser_status_flag(value, "controller"))
                    {
                        Self::render_room_browser_chip(
                            ui,
                            "Controller",
                            ui.visuals().widgets.active.bg_fill.gamma_multiply(0.18),
                            ui.visuals().widgets.active.fg_stroke.color,
                        );
                    }
                    for cue in cues {
                        Self::render_room_browser_chip(
                            ui,
                            cue,
                            ui.visuals().warn_fg_color.gamma_multiply(0.14),
                            ui.visuals().warn_fg_color,
                        );
                    }
                });

                let file_response = ui.add(
                    egui::Label::new(egui::RichText::new(&file_text).small().strong()).truncate(),
                );
                if !file_text.is_empty() && file_text != "(none)" {
                    file_response.on_hover_text(file_text.clone());
                }

                if !metadata.is_empty() {
                    ui.label(egui::RichText::new(metadata).small().weak());
                }

                let visible_actions: Vec<&GuiWidgetNode> = action_nodes
                    .into_iter()
                    .flatten()
                    .filter(|node| {
                        node.enabled
                            || matches!(node.id.as_str(), id if id.ends_with(":ready") || id.ends_with(":open"))
                    })
                    .collect();
                if !visible_actions.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = 6.0;
                        spacing.y = 6.0;
                        ui.spacing_mut().item_spacing = spacing;
                        for action in visible_actions {
                            self.render_room_browser_button(ui, action, state);
                        }
                    });
                }
            });
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
    ) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 2))
            .fill(fill)
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
            .join("  •  ")
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

    fn editable_text_value(node: &GuiWidgetNode) -> String {
        node.value.clone().unwrap_or_default()
    }

    fn update_playlist_drop_target_slot(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        row_index: usize,
    ) {
        let Some(pointer_pos) = ui
            .ctx()
            .pointer_hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .filter(|pointer_pos| response.rect.contains(*pointer_pos))
        else {
            return;
        };
        self.playlist_drop_target_slot =
            Self::playlist_drop_slot_for_response(response, row_index, Some(pointer_pos));
    }

    fn render_playlist_drop_indicator(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        row_index: usize,
        playlist_len: usize,
    ) {
        let Some(payload) = response.dnd_hover_payload::<GuiDraggedPlaylistRow>() else {
            return;
        };
        let pointer_pos = ui
            .ctx()
            .pointer_hover_pos()
            .or_else(|| response.interact_pointer_pos());
        let Some(target_slot) =
            Self::playlist_drop_slot_for_response(response, row_index, pointer_pos)
        else {
            return;
        };
        let Some(action) = Self::playlist_row_move_action(payload.index, target_slot, playlist_len)
        else {
            return;
        };

        let stroke = ui.visuals().widgets.active.fg_stroke;
        let y = if target_slot == row_index {
            response.rect.top()
        } else {
            response.rect.bottom()
        };
        ui.painter().line_segment(
            [
                egui::pos2(response.rect.left(), y),
                egui::pos2(response.rect.right(), y),
            ],
            stroke,
        );

        if response
            .dnd_release_payload::<GuiDraggedPlaylistRow>()
            .is_some()
        {
            self.actions.push(action);
        }
    }

    fn playlist_drop_slot_for_response(
        response: &egui::Response,
        row_index: usize,
        pointer_pos: Option<egui::Pos2>,
    ) -> Option<usize> {
        let pointer_pos = pointer_pos?;
        Some(if pointer_pos.y < response.rect.center().y {
            row_index
        } else {
            row_index.saturating_add(1)
        })
    }

    fn playlist_row_pointer_actions(
        index: usize,
        clicked: bool,
        double_clicked: bool,
    ) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        if clicked || double_clicked {
            actions.push(GuiShellAction::SelectMainWindowPlaylist(index));
        }
        if double_clicked {
            actions.push(GuiShellAction::ActivateMainWindowPlaylist(index));
        }
        actions
    }

    fn playlist_focus_id() -> egui::Id {
        egui::Id::new("main-window:playlist:keyboard")
    }

    fn playlist_focus_sense() -> egui::Sense {
        egui::Sense::focusable_noninteractive()
    }

    fn playlist_row_sense(can_drag_reorder: bool) -> egui::Sense {
        if can_drag_reorder {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        }
    }

    pub(super) fn playlist_row_move_action(
        from_index: usize,
        target_slot: usize,
        playlist_len: usize,
    ) -> Option<GuiShellAction> {
        if from_index >= playlist_len || target_slot > playlist_len {
            return None;
        }
        let to_index = if target_slot > from_index {
            target_slot.saturating_sub(1)
        } else {
            target_slot
        };
        (from_index != to_index).then_some(GuiShellAction::MoveMainWindowPlaylistRow {
            from_index,
            to_index,
        })
    }

    pub(super) fn playlist_row_shortcut_actions(
        state: &SyncplayGuiShellAppState,
        index: usize,
        row_enabled: bool,
        playlist_focused: bool,
        enter_pressed: bool,
        delete_pressed: bool,
    ) -> Vec<GuiShellAction> {
        if !row_enabled
            || !playlist_focused
            || state.selection.selected_main_window_playlist != Some(index)
        {
            return Vec::new();
        }

        if delete_pressed
            && state.pending_operation.is_none()
            && state.main_window.playback.can_manage_playlist
            && state.selection.selected_main_window_playlist == Some(index)
        {
            return vec![GuiShellAction::RemoveSelectedMainWindowPlaylist];
        }

        if enter_pressed {
            return vec![GuiShellAction::ActivateMainWindowPlaylist(index)];
        }

        Vec::new()
    }

    fn render_text_input(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let mut value = Self::editable_text_value(node);
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
                let mut value = Self::editable_text_value(node);
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
        if Self::playback_control_icon(node).is_some() {
            self.render_playback_icon_button(ui, node, state);
            return;
        }
        if node.id == "main-window:control:set-ready" {
            self.render_playback_ready_button(ui, node, state);
            return;
        }
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
            self.handle_button_node_click(state, node);
        }
    }

    fn render_playback_icon_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(icon) = Self::playback_control_icon(node) else {
            return;
        };
        let clicked = ui
            .push_id(&node.id, |ui| {
                let response = ui.add_enabled(
                    node.enabled,
                    egui::Button::new("")
                        .min_size(Self::playback_button_size(ui))
                        .corner_radius(6),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        response.enabled(),
                        node.label.clone(),
                    )
                });
                let response = Self::attach_hover_text(response, node.label.clone());
                Self::paint_playback_control_icon(ui, &response, icon);
                response.clicked()
            })
            .inner;
        if clicked {
            self.handle_button_node_click(state, node);
        }
    }

    fn render_playback_ready_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let is_ready = Self::main_window_display_ready(state);
        let pending = state.local_ready_transition_pending();
        let label = if is_ready { "Ready" } else { "Not Ready" };
        let hover_text = if pending {
            "Updating readiness..."
        } else if is_ready {
            "Mark Not Ready"
        } else {
            "Mark Ready"
        };
        let clicked = ui
            .push_id(&node.id, |ui| {
                let mut clicked = false;
                ui.horizontal(|ui| {
                    let button_width = Self::playback_ready_button_width(ui.available_width());
                    let side_space = ((ui.available_width() - button_width).max(0.0)) * 0.5;
                    if side_space > 0.0 {
                        ui.add_space(side_space);
                    }
                    let response = ui.add_enabled(
                        node.enabled,
                        egui::Button::new("")
                            .frame(false)
                            .corner_radius(18)
                            .min_size(egui::vec2(button_width, 36.0)),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            response.enabled(),
                            label,
                        )
                    });
                    let response = Self::attach_hover_text(response, hover_text);
                    Self::paint_playback_ready_button(ui, &response, label, is_ready, pending);
                    clicked = response.clicked();
                });
                clicked
            })
            .inner;
        if clicked {
            self.handle_button_node_click(state, node);
        }
    }

    fn handle_button_node_click(&mut self, state: &SyncplayGuiShellAppState, node: &GuiWidgetNode) {
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

    fn playback_control_icon(node: &GuiWidgetNode) -> Option<GuiPlaybackControlIcon> {
        match node.id.as_str() {
            "main-window:control:play" => Some(GuiPlaybackControlIcon::Play),
            "main-window:control:pause" => Some(GuiPlaybackControlIcon::Pause),
            "main-window:control:toggle-pause" => Some(GuiPlaybackControlIcon::TogglePause),
            "main-window:control:seek" => Some(GuiPlaybackControlIcon::Seek),
            "main-window:control:undo-seek" => Some(GuiPlaybackControlIcon::UndoSeek),
            "main-window:control:set-offset" => Some(GuiPlaybackControlIcon::SetOffset),
            _ => None,
        }
    }

    fn main_window_display_ready(state: &SyncplayGuiShellAppState) -> bool {
        state.displayed_local_main_window_user_ready()
    }

    fn playback_button_size(ui: &egui::Ui) -> egui::Vec2 {
        egui::vec2(
            ui.available_width().max(0.0),
            ui.available_height().max(ui.spacing().interact_size.y),
        )
    }

    fn playback_ready_button_width(available_width: f32) -> f32 {
        let available_width = available_width.max(0.0);
        let preferred_width = 176.0;
        if available_width >= preferred_width {
            preferred_width
        } else {
            available_width
        }
    }

    fn attach_hover_text(
        response: egui::Response,
        hover_text: impl Into<String>,
    ) -> egui::Response {
        let hover_text = hover_text.into();
        response
            .on_hover_text(hover_text.clone())
            .on_disabled_hover_text(hover_text)
    }

    fn paint_playback_control_icon(
        ui: &egui::Ui,
        response: &egui::Response,
        icon: GuiPlaybackControlIcon,
    ) {
        let visuals = ui.style().interact(response);
        let stroke = egui::Stroke::new(visuals.fg_stroke.width.max(1.75), visuals.fg_stroke.color);
        let fill = visuals.fg_stroke.color;
        let painter = ui.painter();
        let rect = response.rect.shrink2(egui::vec2(10.0, 8.0));
        match icon {
            GuiPlaybackControlIcon::Play => {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(rect.left(), rect.top()),
                        egui::pos2(rect.right(), rect.center().y),
                        egui::pos2(rect.left(), rect.bottom()),
                    ],
                    fill,
                    egui::Stroke::NONE,
                ));
            }
            GuiPlaybackControlIcon::Pause => {
                let bar_width = rect.width() * 0.24;
                let gap = rect.width() * 0.18;
                let left = rect.center().x - ((bar_width * 2.0) + gap) / 2.0;
                let first_bar = egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(left + bar_width, rect.bottom()),
                );
                let second_bar = first_bar.translate(egui::vec2(bar_width + gap, 0.0));
                painter.rect_filled(first_bar, 1.5, fill);
                painter.rect_filled(second_bar, 1.5, fill);
            }
            GuiPlaybackControlIcon::TogglePause => {
                let bar_width = rect.width() * 0.16;
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.left() + bar_width, rect.bottom()),
                );
                let triangle_left = rect.left() + (rect.width() * 0.30);
                painter.rect_filled(bar_rect, 1.5, fill);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(triangle_left, rect.top()),
                        egui::pos2(rect.right(), rect.center().y),
                        egui::pos2(triangle_left, rect.bottom()),
                    ],
                    fill,
                    egui::Stroke::NONE,
                ));
            }
            GuiPlaybackControlIcon::Seek => {
                let mid_x = rect.center().x - 1.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(rect.left(), rect.top()),
                        egui::pos2(mid_x, rect.center().y),
                        egui::pos2(rect.left(), rect.bottom()),
                    ],
                    fill,
                    egui::Stroke::NONE,
                ));
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(mid_x, rect.top()),
                        egui::pos2(rect.right(), rect.center().y),
                        egui::pos2(mid_x, rect.bottom()),
                    ],
                    fill,
                    egui::Stroke::NONE,
                ));
            }
            GuiPlaybackControlIcon::UndoSeek => {
                let arrow_tip = egui::pos2(rect.left() + (rect.width() * 0.14), rect.top() + 6.0);
                let path = vec![
                    egui::pos2(rect.right(), rect.bottom() - 2.0),
                    egui::pos2(rect.center().x + 2.0, rect.bottom() - 2.0),
                    egui::pos2(rect.center().x + 2.0, arrow_tip.y),
                    egui::pos2(rect.left() + 8.0, arrow_tip.y),
                ];
                painter.add(egui::Shape::line(path, stroke));
                painter.line_segment(
                    [egui::pos2(rect.left() + 10.0, rect.top()), arrow_tip],
                    stroke,
                );
                painter.line_segment(
                    [egui::pos2(rect.left() + 10.0, rect.top() + 12.0), arrow_tip],
                    stroke,
                );
            }
            GuiPlaybackControlIcon::SetOffset => {
                let radius = rect.width().min(rect.height()) * 0.46;
                let center = rect.center();
                painter.circle_stroke(center, radius, stroke);
                painter.line_segment(
                    [center, egui::pos2(center.x, center.y - (radius * 0.48))],
                    stroke,
                );
                painter.line_segment(
                    [
                        center,
                        egui::pos2(center.x + (radius * 0.38), center.y + (radius * 0.18)),
                    ],
                    stroke,
                );
            }
        }
    }

    fn paint_playback_ready_button(
        ui: &egui::Ui,
        response: &egui::Response,
        label: &str,
        is_ready: bool,
        pending: bool,
    ) {
        if pending {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        let widget_visuals = &ui.visuals().widgets;
        let visuals = if response.enabled() {
            if response.is_pointer_button_down_on() {
                &widget_visuals.active
            } else if response.hovered() || response.has_focus() {
                &widget_visuals.hovered
            } else {
                &widget_visuals.inactive
            }
        } else {
            &widget_visuals.noninteractive
        };
        let accent_fill = ui.visuals().selection.bg_fill;
        let accent_color = ui.visuals().selection.stroke.color;
        let fill = if is_ready {
            if response.enabled() {
                if response.hovered() {
                    accent_fill.linear_multiply(0.95)
                } else {
                    accent_fill.linear_multiply(0.80)
                }
            } else {
                accent_fill.linear_multiply(0.35)
            }
        } else if response.enabled() && response.hovered() {
            visuals.bg_fill.linear_multiply(1.05)
        } else {
            visuals.bg_fill
        };
        let stroke_color = if is_ready {
            accent_color
        } else {
            visuals.bg_stroke.color
        };
        let stroke = egui::Stroke::new(visuals.bg_stroke.width.max(1.0), stroke_color);
        let text_color = if is_ready {
            accent_color
        } else {
            visuals.fg_stroke.color
        };
        let indicator_fill = if is_ready && !pending {
            text_color
        } else {
            egui::Color32::TRANSPARENT
        };
        let indicator_stroke = egui::Stroke::new(1.5, stroke_color);
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter()
            .rect(rect, 18, fill, stroke, egui::StrokeKind::Inside);

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(label.to_owned(), font_id, text_color);
        let dot_radius = 4.0;
        let gap = 8.0;
        let total_width = (dot_radius * 2.0) + gap + galley.size().x;
        let content_left = rect.center().x - (total_width * 0.5);
        let dot_center = egui::pos2(content_left + dot_radius, rect.center().y);
        let text_pos = egui::pos2(
            content_left + (dot_radius * 2.0) + gap,
            rect.center().y - (galley.size().y * 0.5),
        );
        if pending {
            let time = ui.input(|input| input.time) as f32;
            let spinner_radius = dot_radius + 1.5;
            let start_angle = time * 6.0;
            let sweep = std::f32::consts::PI * 1.25;
            let segment_count = 16;
            let mut spinner_points = Vec::with_capacity(segment_count + 1);
            for index in 0..=segment_count {
                let fraction = index as f32 / segment_count as f32;
                let angle = start_angle + (sweep * fraction);
                spinner_points.push(egui::pos2(
                    dot_center.x + (spinner_radius * angle.cos()),
                    dot_center.y + (spinner_radius * angle.sin()),
                ));
            }
            ui.painter().circle_stroke(
                dot_center,
                spinner_radius + 1.0,
                egui::Stroke::new(1.0, stroke_color.linear_multiply(0.15)),
            );
            ui.painter().add(egui::Shape::line(
                spinner_points,
                egui::Stroke::new(2.0, stroke_color),
            ));
        } else {
            ui.painter()
                .circle(dot_center, dot_radius, indicator_fill, indicator_stroke);
        }
        ui.painter().galley(text_pos, galley, text_color);
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

    pub(super) fn modal_window_title(modal: GuiShellModal) -> &'static str {
        match modal {
            GuiShellModal::TlsCertificatePrompt => "TLS Certificate Prompt",
            GuiShellModal::UpdateNotice => "Update Notice",
            GuiShellModal::About => "About Syncplay",
            GuiShellModal::PlayerSetup => "mpv Setup Required",
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
            GuiShellModal::PlayerSetup => {
                let mut lines = vec![
                    state
                        .player_setup_issue_title()
                        .unwrap_or("mpv setup issue")
                        .to_owned(),
                    state
                        .player_setup_issue_summary()
                        .unwrap_or("Syncplay needs mpv before playback can start.")
                        .to_owned(),
                ];
                if let Some(issue) = state.player_setup_issue.as_ref() {
                    lines.push(issue.message.clone());
                }
                if state.connect_blocked_by_player_setup_issue()
                    && let Some(message) = state.player_setup_connect_block_message()
                {
                    lines.push(message);
                }
                lines
            }
        }
    }

    pub(super) fn modal_actions(modal: GuiShellModal) -> Vec<(&'static str, &'static str)> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                ("shell:modal:tls:trust", "Trust Certificate"),
                ("shell:modal:tls:reject", "Reject Certificate"),
                ("shell:modal:tls:help", "Open Help"),
            ],
            GuiShellModal::UpdateNotice => vec![
                ("shell:modal:update:dismiss", "Dismiss Notice"),
                ("shell:modal:update:help", "Open Help"),
                ("shell:modal:update:check-again", "Check Again"),
            ],
            GuiShellModal::About => vec![
                ("shell:modal:about:help", "Open Help"),
                ("shell:modal:about:update", "Check for Updates"),
            ],
            GuiShellModal::PlayerSetup => vec![
                ("shell:modal:player-setup:autodetect", "Auto-detect mpv"),
                ("shell:modal:player-setup:choose-path", "Choose mpv.exe"),
                ("shell:modal:player-setup:retry", "Retry mpv"),
                ("shell:modal:player-setup:open-settings", "Open Settings"),
            ],
        }
    }

    pub(super) fn modal_action_enabled(state: &SyncplayGuiShellAppState, id: &str) -> bool {
        match id {
            "shell:modal:player-setup:autodetect"
            | "shell:modal:player-setup:choose-path"
            | "shell:modal:player-setup:open-settings" => state.pending_operation.is_none(),
            "shell:modal:player-setup:retry" => {
                state.pending_operation.is_none() && state.player_setup_retry_available()
            }
            _ => true,
        }
    }

    pub(super) fn modal_close_enabled(
        state: &SyncplayGuiShellAppState,
        modal: GuiShellModal,
    ) -> bool {
        modal != GuiShellModal::PlayerSetup || !state.connect_blocked_by_player_setup_issue()
    }

    fn modal_button_actions(
        state: &SyncplayGuiShellAppState,
        id: &str,
        label: &str,
    ) -> Vec<GuiShellAction> {
        let node = GuiWidgetNode::leaf(id, label, GuiWidgetKind::Button, None, true, false);
        Self::actions_for_clicked_button(state, &node)
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
