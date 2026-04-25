use eframe::egui;

use super::render_io::GuiDroppedFilesRequest;
use super::shell_state::{GuiShellAction, GuiShellModal, SyncplayGuiShellAppState};
use super::widget_tree::{GuiWidgetKind, GuiWidgetNode};

mod chat;
mod controls;
mod display;
mod layout;
mod modal;
mod playback_controls;
mod playlist;
mod room_browser;
#[cfg(test)]
mod tests;
mod tree_renderer;

#[derive(Debug, Default)]
pub(super) struct GuiWidgetEguiRenderer {
    stack: Vec<GuiWidgetNode>,
    root: Option<GuiWidgetNode>,
    actions: Vec<GuiShellAction>,
    close_requested: bool,
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
pub(super) enum GuiRoomDashboardLayout {
    Narrow,
    Medium,
    Wide,
}

#[derive(Debug, Clone, Copy)]
struct GuiSemanticPalette {
    primary: egui::Color32,
    primary_hover: egui::Color32,
    primary_text: egui::Color32,
    danger: egui::Color32,
    danger_hover: egui::Color32,
    danger_text: egui::Color32,
    success_text: egui::Color32,
    success_bg: egui::Color32,
    success_border: egui::Color32,
    warning_text: egui::Color32,
    warning_bg: egui::Color32,
    warning_border: egui::Color32,
    info_text: egui::Color32,
    info_bg: egui::Color32,
    info_border: egui::Color32,
    controlled_text: egui::Color32,
    controlled_bg: egui::Color32,
    controlled_border: egui::Color32,
    neutral_text: egui::Color32,
    neutral_border: egui::Color32,
}

impl GuiWidgetEguiRenderer {
    fn palette_for_ui(ui: &egui::Ui) -> GuiSemanticPalette {
        Self::palette_for_dark_mode(ui.visuals().dark_mode)
    }

    fn palette_for_dark_mode(dark_mode: bool) -> GuiSemanticPalette {
        if dark_mode {
            return GuiSemanticPalette {
                primary: egui::Color32::from_rgb(89, 147, 191),
                primary_hover: egui::Color32::from_rgb(104, 164, 210),
                primary_text: egui::Color32::from_rgb(11, 18, 26),
                danger: egui::Color32::from_rgb(218, 120, 112),
                danger_hover: egui::Color32::from_rgb(236, 142, 133),
                danger_text: egui::Color32::from_rgb(24, 10, 10),
                success_text: egui::Color32::from_rgb(118, 210, 156),
                success_bg: egui::Color32::from_rgb(24, 60, 42),
                success_border: egui::Color32::from_rgb(68, 139, 95),
                warning_text: egui::Color32::from_rgb(236, 190, 107),
                warning_bg: egui::Color32::from_rgb(69, 52, 23),
                warning_border: egui::Color32::from_rgb(154, 118, 47),
                info_text: egui::Color32::from_rgb(141, 203, 234),
                info_bg: egui::Color32::from_rgb(28, 58, 72),
                info_border: egui::Color32::from_rgb(79, 142, 171),
                controlled_text: egui::Color32::from_rgb(200, 181, 238),
                controlled_bg: egui::Color32::from_rgb(54, 43, 77),
                controlled_border: egui::Color32::from_rgb(126, 103, 173),
                neutral_text: egui::Color32::from_rgb(226, 232, 240),
                neutral_border: egui::Color32::from_rgb(84, 98, 118),
            };
        }

        GuiSemanticPalette {
            primary: egui::Color32::from_rgb(65, 111, 148),
            primary_hover: egui::Color32::from_rgb(52, 91, 123),
            primary_text: egui::Color32::WHITE,
            danger: egui::Color32::from_rgb(155, 83, 77),
            danger_hover: egui::Color32::from_rgb(130, 67, 62),
            danger_text: egui::Color32::WHITE,
            success_text: egui::Color32::from_rgb(48, 119, 80),
            success_bg: egui::Color32::from_rgb(235, 247, 240),
            success_border: egui::Color32::from_rgb(139, 196, 162),
            warning_text: egui::Color32::from_rgb(132, 94, 28),
            warning_bg: egui::Color32::from_rgb(255, 248, 230),
            warning_border: egui::Color32::from_rgb(212, 178, 90),
            info_text: egui::Color32::from_rgb(55, 101, 125),
            info_bg: egui::Color32::from_rgb(240, 247, 250),
            info_border: egui::Color32::from_rgb(132, 175, 196),
            controlled_text: egui::Color32::from_rgb(102, 86, 137),
            controlled_bg: egui::Color32::from_rgb(244, 241, 250),
            controlled_border: egui::Color32::from_rgb(167, 154, 199),
            neutral_text: egui::Color32::from_rgb(55, 65, 81),
            neutral_border: egui::Color32::from_rgb(188, 196, 207),
        }
    }

    pub(super) fn root(&self) -> Option<&GuiWidgetNode> {
        self.root.as_ref()
    }

    pub(super) fn take_close_requested(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
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
        let stream_helper_remediation_active = root
            .find("shell:stream-helper-remediation-active")
            .and_then(|node| node.value.as_deref())
            .is_some_and(|value| matches!(value, "yes" | "true"));
        let stream_helper_remediation_label = root
            .find("shell:stream-helper-remediation-label")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(idle)");
        let stream_helper_remediation_detail = root
            .find("shell:stream-helper-remediation-detail")
            .and_then(|node| node.value.as_deref())
            .unwrap_or("(idle)");
        let stream_helper_remediation_progress = root
            .find("shell:stream-helper-remediation-progress")
            .and_then(|node| node.value.as_deref())
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let show_manual_controls = Self::should_show_manual_pending_controls(
            pending_operation,
            show_manual_pending_controls,
        );
        let show_visible_status = media_index_active
            || stream_helper_remediation_active
            || show_manual_controls
            || pending_operation != "(none)";
        let mut panel = egui::TopBottomPanel::bottom("syncplay-native-status-bar");
        if !show_visible_status {
            panel = panel.exact_height(1.0).frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(0))
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE),
            );
        }
        panel.show(ctx, |ui| {
            Self::render_status_accessibility_markers(
                ui,
                active_view,
                open_modal,
                pending_operation,
            );
            if !show_visible_status {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.strong("Syncplay");
                if pending_operation != "(none)" {
                    ui.separator();
                    ui.label(format!("Pending: {pending_operation}"));
                }
                if media_index_active {
                    ui.separator();
                    ui.add(egui::Spinner::new());
                    ui.label(media_index_status);
                }
                if stream_helper_remediation_active {
                    ui.separator();
                    ui.label(stream_helper_remediation_label);
                    ui.add(
                        egui::ProgressBar::new(stream_helper_remediation_progress)
                            .desired_width(160.0)
                            .show_percentage(),
                    );
                    if stream_helper_remediation_detail != "(idle)" {
                        ui.label(stream_helper_remediation_detail);
                    }
                }
                if show_manual_controls {
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

    fn render_status_accessibility_markers(
        ui: &mut egui::Ui,
        active_view: &str,
        open_modal: &str,
        pending_operation: &str,
    ) {
        for label in [
            format!("view: {active_view}"),
            format!("modal: {open_modal}"),
            format!("pending: {pending_operation}"),
        ] {
            Self::render_accessibility_marker(ui, label);
        }
    }

    fn render_accessibility_marker(ui: &mut egui::Ui, label: impl Into<String>) {
        let marker_label = label.into();
        let (_, response) = ui.allocate_exact_size(egui::vec2(1.0, 1.0), egui::Sense::hover());
        response.widget_info(move || {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, marker_label.clone())
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
        _state: &SyncplayGuiShellAppState,
    ) {
        egui::SidePanel::left("syncplay-native-navigation")
            .default_width(118.0)
            .min_width(104.0)
            .max_width(132.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                for child in &root.children {
                    if Self::is_surface_node(child) {
                        let response = Self::render_navigation_button(ui, child);
                        if response.clicked()
                            && let Some(action) = Self::action_for_surface_node(child)
                        {
                            self.actions.push(action);
                        }
                        ui.add_space(6.0);
                    }
                }
                Self::render_shell_accessibility_markers(ui, root);
            });
    }

    fn render_shell_accessibility_markers(ui: &mut egui::Ui, root: &GuiWidgetNode) {
        for branch_id in ["shell:commands", "shell:validation", "shell:notifications"] {
            let Some(branch) = root.find(branch_id) else {
                continue;
            };
            for child in &branch.children {
                Self::render_accessibility_marker(ui, Self::display_text(child));
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

    fn render_navigation_button(ui: &mut egui::Ui, node: &GuiWidgetNode) -> egui::Response {
        let desired_size = egui::vec2(ui.available_width().max(88.0), 52.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &node.label)
        });

        let visuals = ui.style().interact(&response);
        let selection = &ui.visuals().selection;
        let fill = if node.selected {
            selection.bg_fill
        } else if response.hovered() {
            visuals.bg_fill.linear_multiply(1.08)
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = if node.selected {
            selection.stroke
        } else if response.hovered() {
            visuals.bg_stroke
        } else {
            egui::Stroke::NONE
        };
        let text_color = if node.selected {
            selection.stroke.color
        } else {
            visuals.text_color()
        };
        let button_rect = rect.shrink2(egui::vec2(6.0, 3.0));
        ui.painter()
            .rect(button_rect, 6, fill, stroke, egui::StrokeKind::Inside);

        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(button_rect.left() + 10.0, button_rect.center().y - 10.0),
            egui::vec2(20.0, 20.0),
        );
        Self::paint_navigation_icon(ui, icon_rect, node.id.as_str(), text_color);

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(node.label.clone(), font_id, text_color);
        let text_pos = egui::pos2(
            icon_rect.right() + 10.0,
            button_rect.center().y - (galley.size().y * 0.5),
        );
        ui.painter()
            .with_clip_rect(button_rect)
            .galley(text_pos, galley, text_color);

        response
    }

    fn paint_navigation_icon(ui: &egui::Ui, rect: egui::Rect, node_id: &str, color: egui::Color32) {
        let painter = ui.painter();
        let stroke = egui::Stroke::new(2.0, color);
        if node_id == "main-window-root" {
            let roof = vec![
                egui::pos2(rect.left() + 1.0, rect.center().y),
                egui::pos2(rect.center().x, rect.top() + 2.0),
                egui::pos2(rect.right() - 1.0, rect.center().y),
            ];
            painter.add(egui::Shape::line(roof, stroke));
            let body = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, rect.center().y - 1.0),
                egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
            );
            painter.rect_stroke(body, 2, stroke, egui::StrokeKind::Inside);
        } else {
            for (index, y_fraction) in [0.25_f32, 0.50, 0.75].into_iter().enumerate() {
                let y = rect.top() + rect.height() * y_fraction;
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 2.0, y),
                        egui::pos2(rect.right() - 2.0, y),
                    ],
                    stroke,
                );
                let x = if index.is_multiple_of(2) {
                    rect.left() + rect.width() * 0.35
                } else {
                    rect.left() + rect.width() * 0.65
                };
                painter.circle_filled(egui::pos2(x, y), 3.0, color);
            }
        }
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
        if node.id == "main-window:top-region" {
            self.render_room_dashboard(ui, node, state);
            return;
        }
        if node.id == "main-window:chat-compose" {
            self.render_chat_compose(ui, node, state);
            return;
        }
        if node.id == "main-window:playlist-header:actions" {
            self.render_playlist_header_actions(ui, node, state);
            return;
        }
        if node.id == "config-commands" {
            self.render_setup_command_bar(ui, node, state);
            return;
        }
        match node.kind {
            GuiWidgetKind::Layout => self.render_layout(ui, node, state),
            GuiWidgetKind::Panel => self.render_panel(ui, node, state),
            GuiWidgetKind::List => {
                if node.id == "main-window:playlist" {
                    self.render_playlist_list(ui, node, state);
                    return;
                }
                if node.id == "main-window:chat" {
                    self.render_chat_history(ui, node);
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

    fn render_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(0))
            .show(ui, |ui| {
                if let Some(min_content_height) = node.min_content_height {
                    ui.set_min_height(min_content_height);
                }
                let close_button = node.children.iter().find(|child| {
                    child.kind == GuiWidgetKind::Button && child.id.ends_with(":close")
                });
                self.render_panel_header(ui, node, close_button, state);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        for child in node.children.iter().filter(|child| {
                            !(child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"))
                        }) {
                            self.render_node(ui, child, state);
                        }
                    });
            });
    }

    fn render_panel_header(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        close_button: Option<&GuiWidgetNode>,
        state: &SyncplayGuiShellAppState,
    ) {
        let header_width = ui.available_width().max(0.0);
        let palette = Self::palette_for_ui(ui);
        egui::Frame::new()
            .fill(Self::panel_header_fill(ui))
            .stroke(Self::panel_header_stroke(ui))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(header_width);
                ui.horizontal(|ui| {
                    ui.strong(egui::RichText::new(&node.label).color(palette.neutral_text));
                    if node.selected {
                        ui.label(egui::RichText::new("active").small().strong());
                    }
                    if !node.enabled {
                        ui.label(egui::RichText::new("disabled").small());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(close_button) = close_button {
                            self.render_panel_close_button(ui, close_button, state);
                        }
                    });
                });
            });
    }

    fn panel_header_fill(ui: &egui::Ui) -> egui::Color32 {
        if ui.visuals().dark_mode {
            egui::Color32::from_rgb(38, 45, 54)
        } else {
            egui::Color32::from_rgb(248, 249, 251)
        }
    }

    fn panel_header_stroke(ui: &egui::Ui) -> egui::Stroke {
        egui::Stroke::new(
            1.0,
            Self::palette_for_ui(ui).neutral_border.gamma_multiply(0.75),
        )
    }

    pub(super) fn room_dashboard_layout_for_width(width: f32) -> GuiRoomDashboardLayout {
        let width = width.max(0.0);
        if width < 760.0 {
            GuiRoomDashboardLayout::Narrow
        } else if width < 1200.0 {
            GuiRoomDashboardLayout::Medium
        } else {
            GuiRoomDashboardLayout::Wide
        }
    }

    fn render_panel_close_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) -> bool {
        let response = ui.add_enabled(
            node.enabled,
            egui::Button::new("")
                .frame(false)
                .min_size(egui::vec2(36.0, 36.0))
                .corner_radius(18),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &node.label)
        });
        let response = Self::attach_hover_text(response, node.label.clone());
        Self::paint_panel_close_button(ui, &response);
        let clicked = response.clicked();
        if clicked {
            self.handle_button_node_click(state, node);
        }
        clicked
    }

    fn paint_panel_close_button(ui: &egui::Ui, response: &egui::Response) {
        let accent = ui.visuals().warn_fg_color;
        let fill = if response.is_pointer_button_down_on() {
            accent.gamma_multiply(0.28)
        } else if response.hovered() {
            accent.gamma_multiply(0.22)
        } else {
            accent.gamma_multiply(0.14)
        };
        let stroke = egui::Stroke::new(
            if response.hovered() { 1.6 } else { 1.2 },
            accent.gamma_multiply(if response.enabled() { 0.92 } else { 0.45 }),
        );
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter()
            .rect(rect, 18, fill, stroke, egui::StrokeKind::Inside);

        let line_stroke = egui::Stroke::new(
            if response.hovered() { 2.3 } else { 2.0 },
            accent.gamma_multiply(if response.enabled() { 1.0 } else { 0.55 }),
        );
        let icon_rect = rect.shrink2(egui::vec2(11.0, 11.0));
        ui.painter().line_segment(
            [icon_rect.left_top(), icon_rect.right_bottom()],
            line_stroke,
        );
        ui.painter().line_segment(
            [icon_rect.right_top(), icon_rect.left_bottom()],
            line_stroke,
        );
    }

    fn truncate_single_line_text_for_width(
        ui: &egui::Ui,
        text: &str,
        font_id: egui::FontId,
        text_color: egui::Color32,
        max_width: f32,
    ) -> (String, bool) {
        if text.is_empty() || max_width <= 0.0 {
            return (String::new(), !text.is_empty());
        }

        let painter = ui.painter();
        let fits = |candidate: &str| {
            painter
                .layout_no_wrap(candidate.to_owned(), font_id.clone(), text_color)
                .size()
                .x
                <= max_width
        };

        if fits(text) {
            return (text.to_owned(), false);
        }

        let ellipsis = "...";
        if !fits(ellipsis) {
            return (String::new(), true);
        }

        let mut boundaries = text
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(text.len());

        let mut low = 0usize;
        let mut high = boundaries.len().saturating_sub(1);
        let mut best = 0usize;
        while low <= high {
            let mid = low + ((high - low) / 2);
            let candidate = format!("{}{ellipsis}", &text[..boundaries[mid]]);
            if fits(&candidate) {
                best = mid;
                low = mid.saturating_add(1);
            } else if mid == 0 {
                break;
            } else {
                high = mid - 1;
            }
        }

        (format!("{}{ellipsis}", &text[..boundaries[best]]), true)
    }
}
