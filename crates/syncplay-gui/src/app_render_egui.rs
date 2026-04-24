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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiCompactActionIcon {
    Add,
    More,
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
    neutral_bg: egui::Color32,
    neutral_border: egui::Color32,
}

impl GuiWidgetEguiRenderer {
    fn palette() -> GuiSemanticPalette {
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
            neutral_bg: egui::Color32::from_rgb(238, 240, 243),
            neutral_border: egui::Color32::from_rgb(188, 196, 207),
        }
    }

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
        let palette = Self::palette();
        egui::Frame::new()
            .fill(Self::panel_header_fill(ui))
            .stroke(Self::panel_header_stroke())
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

    fn panel_header_stroke() -> egui::Stroke {
        egui::Stroke::new(1.0, Self::palette().neutral_border.gamma_multiply(0.75))
    }

    fn render_room_dashboard(
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

        let available_width = ui.available_width().max(0.0).min(1420.0);
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

    fn render_playlist_header_actions(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        ui.horizontal_top(|ui| {
            let mut spacing = ui.spacing().item_spacing;
            spacing.x = Self::COMPACT_ACTION_BUTTON_GAP;
            ui.spacing_mut().item_spacing = spacing;
            for child in &node.children {
                self.render_compact_action_button(ui, child, state);
            }
        });
        ui.add_space(8.0);
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
            ui.set_min_width(ui.available_width().max(0.0));
            if let Some(min_content_height) = node.min_content_height {
                ui.set_min_height(min_content_height);
            }
            if node.children.is_empty() {
                Self::paint_empty_playlist_state(ui, node);
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
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                response.response.enabled(),
                node.label.clone(),
            )
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

    fn paint_empty_playlist_state(ui: &mut egui::Ui, node: &GuiWidgetNode) {
        let row_height = 38.0;
        let available_width = ui.available_width().max(0.0);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(available_width, row_height),
            egui::Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                response.enabled(),
                "Playlist is empty.",
            )
        });

        let visuals = ui.visuals();
        let fill = visuals.widgets.noninteractive.bg_fill.gamma_multiply(0.08);
        ui.painter().rect(
            rect.shrink2(egui::vec2(0.5, 0.5)),
            2,
            fill,
            visuals.widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );

        let text = if node.enabled {
            "Playlist is empty."
        } else {
            "Playlist unavailable."
        };
        let text_color = visuals.weak_text_color();
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font_id, text_color);
        ui.painter().galley(
            egui::pos2(
                rect.left() + 12.0,
                rect.center().y - (galley.size().y * 0.5),
            ),
            galley,
            text_color,
        );
    }

    fn render_chat_history(&mut self, ui: &mut egui::Ui, node: &GuiWidgetNode) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let history_height = node.min_content_height.unwrap_or(180.0).clamp(120.0, 260.0);
            ui.set_min_width(ui.available_width().max(0.0));
            ui.set_min_height(history_height);
            ui.set_max_height(history_height);
            egui::ScrollArea::vertical()
                .id_salt(&node.id)
                .max_height(history_height)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width().max(0.0));
                    if node.children.is_empty() {
                        Self::paint_empty_chat_history_state(ui);
                        return;
                    }
                    for (index, child) in node.children.iter().enumerate() {
                        Self::paint_chat_history_row(ui, child, index);
                    }
                });
        });
    }

    fn paint_empty_chat_history_state(ui: &mut egui::Ui) {
        let row_height = 40.0;
        let available_width = ui.available_width().max(0.0);
        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(available_width, row_height),
            egui::Sense::hover(),
        );
        let text = "No chat messages.";
        let text_color = ui.visuals().weak_text_color();
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), font_id, text_color);
        ui.painter().galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - (galley.size().y * 0.5)),
            galley,
            text_color,
        );
    }

    fn paint_chat_history_row(ui: &mut egui::Ui, node: &GuiWidgetNode, index: usize) {
        let row_height = 28.0;
        let available_width = ui.available_width().max(0.0);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(available_width, row_height),
            egui::Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                response.enabled(),
                Self::display_text(node),
            )
        });
        let fill = if index.is_multiple_of(2) {
            ui.visuals()
                .widgets
                .noninteractive
                .bg_fill
                .gamma_multiply(0.08)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 0, fill);
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.bottom()),
                egui::pos2(rect.right(), rect.bottom()),
            ],
            ui.visuals().widgets.noninteractive.bg_stroke,
        );

        let label = Self::display_text(node);
        let text_color = ui.visuals().text_color();
        let text_left = rect.left() + 8.0;
        let text_width = (rect.right() - text_left - 8.0).max(0.0);
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let (display_label, truncated) = Self::truncate_single_line_text_for_width(
            ui,
            &label,
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
        if truncated {
            response.on_hover_text(label);
        }
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
        let is_room_active = state.main_window.active_playlist_index == Some(index);

        let button_response = ui
            .push_id(&node.id, |ui| {
                ui.add_enabled_ui(node.enabled, |ui| {
                    let response = ui.add_sized(
                        [ui.available_width().max(0.0), 38.0],
                        egui::Button::new("")
                            .frame(false)
                            .sense(Self::playlist_row_sense(can_drag_reorder)),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            response.enabled(),
                            text.clone(),
                        )
                    });
                    let truncated = Self::paint_playlist_row(
                        ui,
                        &response,
                        &text,
                        node.selected,
                        is_room_active,
                    );
                    if truncated {
                        response.on_hover_text(text.clone())
                    } else {
                        response
                    }
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
                .stroke(Self::panel_header_stroke())
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(header_width);
                    ui.horizontal(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(
                                egui::RichText::new(&node.label)
                                    .color(Self::palette().neutral_text),
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
                                let palette = Self::palette();
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
        let palette = Self::palette();
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
                        let palette = Self::palette();
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
                        let palette = Self::palette();
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
        let palette = Self::palette();
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
                                let palette = Self::palette();
                                Self::render_room_browser_chip(
                                    ui,
                                    "You",
                                    palette.info_bg,
                                    palette.info_text,
                                    palette.info_border,
                                );
                            }
                            if is_controller {
                                let palette = Self::palette();
                                Self::render_room_browser_chip(
                                    ui,
                                    "Controller",
                                    palette.controlled_bg,
                                    palette.controlled_text,
                                    palette.controlled_border,
                                );
                            }
                            for cue in &cues {
                                let palette = Self::palette();
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
        let palette = Self::palette();
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

    fn render_chat_compose(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        let input_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:chat-input");
        let send_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:chat:send");
        let Some(input_node) = input_node else {
            return;
        };

        ui.horizontal(|ui| {
            let mut value = Self::editable_text_value(input_node);
            let send_width = 84.0;
            let input_width = (ui.available_width() - send_width - 8.0).max(160.0);
            let response = ui.add_enabled(
                input_node.enabled,
                egui::TextEdit::singleline(&mut value)
                    .id_salt(&input_node.id)
                    .return_key(None)
                    .desired_width(input_width)
                    .hint_text("Type a message..."),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    response.enabled(),
                    input_node.label.clone(),
                )
            });
            let submitted =
                response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if let Some(actions) = Self::actions_for_text_input_node(
                state,
                input_node,
                &value,
                response.changed(),
                submitted,
            ) {
                self.actions.extend(actions);
            }
            if submitted {
                response.request_focus();
            }

            if let Some(send_node) = send_node {
                let mut send_clicked = false;
                ui.allocate_ui_with_layout(
                    egui::vec2(send_width, ui.spacing().interact_size.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(send_width);
                        send_clicked = self.render_button_like(ui, send_node, state);
                    },
                );
                if send_clicked {
                    response.request_focus();
                }
            }
        });
    }

    fn render_setup_command_bar(
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
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    response.enabled(),
                    node.label.clone(),
                )
            });
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
                    ui.horizontal_wrapped(|ui| {
                        let mut spacing = ui.spacing().item_spacing;
                        spacing.x = spacing.x.max(4.0);
                        ui.spacing_mut().item_spacing = spacing;
                        ui.label(egui::RichText::new(format!("{}:", node.label)).strong());
                        ui.label(Self::display_status_rich_text(ui, node));
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

    fn render_button_like(
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
        let mut clicked = false;
        ui.add_enabled_ui(node.enabled, |ui| {
            let mut label = egui::RichText::new(Self::display_text(node));
            if node.enabled
                && let Some((_, _, text_color)) = Self::button_colors_for_node(node)
            {
                label = label.color(text_color).strong();
            }
            let mut button = egui::Button::new(label);
            if node.enabled
                && let Some((fill, hover_fill, _)) = Self::button_colors_for_node(node)
            {
                button = button.fill(
                    if ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
                        hover_fill
                    } else {
                        fill
                    },
                );
            }
            clicked = ui
                .add_sized([ui.available_width().max(0.0), 0.0], button)
                .clicked();
        });
        if clicked {
            self.handle_button_node_click(state, node);
        }
        clicked
    }

    const COMPACT_ACTION_BUTTON_HEIGHT: f32 = 32.0;
    const COMPACT_ACTION_BUTTON_MIN_WIDTH: f32 = 86.0;
    const COMPACT_ACTION_BUTTON_MAX_WIDTH: f32 = 136.0;
    const COMPACT_ACTION_BUTTON_GAP: f32 = 8.0;

    pub(super) fn compact_action_button_size(node: &GuiWidgetNode) -> egui::Vec2 {
        let text_width = Self::display_text(node).chars().count() as f32 * 7.5;
        let icon_width = if Self::compact_action_icon(node).is_some() {
            24.0
        } else {
            0.0
        };
        let menu_indicator_width = if node.children.is_empty() { 0.0 } else { 14.0 };
        let horizontal_padding = 28.0;
        egui::vec2(
            (text_width + icon_width + menu_indicator_width + horizontal_padding).clamp(
                Self::COMPACT_ACTION_BUTTON_MIN_WIDTH,
                Self::COMPACT_ACTION_BUTTON_MAX_WIDTH,
            ),
            Self::COMPACT_ACTION_BUTTON_HEIGHT,
        )
    }

    fn render_compact_action_button(
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
            "main-window:playlist:add-menu" => Some(GuiCompactActionIcon::Add),
            "main-window:playlist:more-menu" => Some(GuiCompactActionIcon::More),
            _ => None,
        }
    }

    fn paint_compact_action_button(
        ui: &egui::Ui,
        response: &egui::Response,
        node: &GuiWidgetNode,
        label: &str,
    ) {
        let palette = Self::palette();
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
            egui::Color32::from_rgb(248, 250, 252)
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

        let mut text_left = rect.left() + 12.0;
        if let Some(icon) = Self::compact_action_icon(node) {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 16.0, rect.center().y),
                egui::vec2(14.0, 14.0),
            );
            Self::paint_compact_action_icon(ui, icon_rect, icon, text_color);
            text_left = icon_rect.right() + 8.0;
        }

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
        match icon {
            GuiCompactActionIcon::Add => {
                let stroke = egui::Stroke::new(1.8, color);
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left(), rect.center().y),
                        egui::pos2(rect.right(), rect.center().y),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.center().x, rect.top()),
                        egui::pos2(rect.center().x, rect.bottom()),
                    ],
                    stroke,
                );
            }
            GuiCompactActionIcon::More => {
                for x_offset in [-4.5_f32, 0.0, 4.5] {
                    ui.painter().circle_filled(
                        egui::pos2(rect.center().x + x_offset, rect.center().y),
                        1.8,
                        color,
                    );
                }
            }
        }
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

    fn button_colors_for_node(
        node: &GuiWidgetNode,
    ) -> Option<(egui::Color32, egui::Color32, egui::Color32)> {
        let palette = Self::palette();
        match node.id.as_str() {
            "main-window:connection:connect"
            | "main-window:room:join"
            | "main-window:room:set"
            | "main-window:room-actions:create-controlled-room"
            | "main-window:room-actions:identify-controller"
            | "main-window:media-url-edit:commit"
            | "main-window:chat:send" => {
                Some((palette.primary, palette.primary_hover, palette.primary_text))
            }
            "main-window:connection:disconnect"
            | "main-window:room:leave"
            | "main-window:media-url-edit:cancel" => {
                Some((palette.danger, palette.danger_hover, palette.danger_text))
            }
            _ => None,
        }
    }

    fn render_playback_icon_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) -> bool {
        let Some(icon) = Self::playback_control_icon(node) else {
            return false;
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
        clicked
    }

    fn render_playback_ready_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) -> bool {
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
        clicked
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
        let palette = Self::palette();
        let fill = if pending {
            palette.info_bg
        } else if is_ready {
            palette.success_bg
        } else if response.enabled() {
            palette.warning_bg
        } else {
            visuals.bg_fill
        };
        let stroke_color = if pending {
            palette.info_border
        } else if is_ready {
            palette.success_border
        } else if response.enabled() {
            palette.warning_border
        } else {
            visuals.bg_stroke.color
        };
        let stroke = egui::Stroke::new(visuals.bg_stroke.width.max(1.0), stroke_color);
        let text_color = if pending {
            palette.info_text
        } else if is_ready {
            palette.success_text
        } else if response.enabled() {
            palette.warning_text
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

    fn paint_playlist_row(
        ui: &egui::Ui,
        response: &egui::Response,
        label: &str,
        is_selected: bool,
        is_room_active: bool,
    ) -> bool {
        let visuals = ui.style().interact(response);
        let palette = Self::palette();
        let active_color = palette.success_text;
        let fill = if is_selected {
            if is_room_active {
                palette.success_bg
            } else {
                palette.info_bg
            }
        } else if is_room_active {
            palette.success_bg.gamma_multiply(0.70)
        } else if response.enabled() && response.hovered() {
            visuals.bg_fill.linear_multiply(1.05)
        } else {
            visuals.bg_fill
        };
        let stroke_color = if is_room_active {
            active_color
        } else if is_selected {
            palette.info_border
        } else {
            visuals.bg_stroke.color
        };
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            2,
            fill,
            egui::Stroke::new(
                if is_room_active {
                    1.5
                } else {
                    visuals.bg_stroke.width.max(1.0)
                },
                stroke_color,
            ),
            egui::StrokeKind::Inside,
        );

        if is_room_active {
            let strip_rect = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + 3.0, rect.bottom()),
            );
            ui.painter().rect_filled(strip_rect, 1, active_color);
        }

        let text_color = if is_selected || is_room_active {
            if is_room_active {
                palette.success_text
            } else {
                palette.info_text
            }
        } else {
            visuals.text_color()
        };
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.center().y - 8.0),
            egui::vec2(16.0, 16.0),
        );
        Self::paint_playlist_file_icon(ui, icon_rect, text_color.gamma_multiply(0.74));

        if is_room_active {
            let icon_center = egui::pos2(icon_rect.right() + 9.0, rect.center().y);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(icon_center.x - 3.0, icon_center.y - 5.0),
                    egui::pos2(icon_center.x + 5.0, icon_center.y),
                    egui::pos2(icon_center.x - 3.0, icon_center.y + 5.0),
                ],
                active_color,
                egui::Stroke::NONE,
            ));
        }

        let text_left = rect.left() + if is_room_active { 50.0 } else { 36.0 };
        let text_right = (rect.right() - 12.0).max(text_left);
        let text_width = (text_right - text_left).max(0.0);
        let (display_label, truncated) = Self::truncate_single_line_text_for_width(
            ui,
            label,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
            text_width,
        );
        let galley = ui.painter().layout_no_wrap(
            display_label,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
        let text_pos = egui::pos2(text_left, rect.center().y - (galley.size().y * 0.5));
        ui.painter()
            .with_clip_rect(rect.shrink2(egui::vec2(8.0, 4.0)))
            .galley(text_pos, galley, text_color);
        truncated
    }

    fn paint_playlist_file_icon(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
        let stroke = egui::Stroke::new(1.6, color);
        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 1.0, rect.top() + 2.0),
            egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        ui.painter()
            .rect_stroke(body, 1, stroke, egui::StrokeKind::Inside);
        ui.painter().line_segment(
            [
                egui::pos2(body.left() + 3.0, body.center().y),
                egui::pos2(body.right() - 3.0, body.center().y),
            ],
            stroke,
        );
        let tab = egui::Rect::from_min_max(
            egui::pos2(body.left() + 2.0, rect.top() + 1.0),
            egui::pos2(body.left() + 8.0, body.top() + 4.0),
        );
        ui.painter().rect_filled(tab, 1, color.gamma_multiply(0.45));
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
            GuiShellModal::StreamSupport => "Stream Support",
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
            GuiShellModal::StreamSupport => {
                let mut lines = vec![
                    state.stream_helper_status_title().to_owned(),
                    state.stream_helper_status_summary(),
                ];
                if let Some(install_location) = state.stream_helper.install_location.as_ref() {
                    lines.push(format!("Install location: {install_location}"));
                }
                if let Some(downloader_status) = state.stream_helper.downloader_status.as_ref() {
                    lines.push(format!("yt-dlp: {downloader_status}"));
                }
                if let Some(js_runtime_status) = state.stream_helper.js_runtime_status.as_ref() {
                    lines.push(format!("Deno: {js_runtime_status}"));
                }
                if let Some(target) = state.stream_helper.target.as_ref() {
                    lines.push(format!("Target: {target}"));
                }
                if let Some(message) = state.stream_helper.message.as_ref() {
                    lines.push(message.clone());
                }
                if state.stream_helper_remediation.active {
                    if let Some(label) = state.stream_helper_remediation.label.as_ref() {
                        lines.push(format!("Progress: {label}"));
                    }
                    if let Some(detail) = state.stream_helper_remediation.detail.as_ref() {
                        lines.push(detail.clone());
                    }
                }
                if state.stream_helper.integration_supported {
                    lines.push(
                        "Import yt-dlp or Deno to copy existing helper binaries into Syncplay's managed stream-helper directory."
                            .to_owned(),
                    );
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
            GuiShellModal::StreamSupport => vec![
                ("shell:modal:stream-support:install", "Install Helper"),
                (
                    "shell:modal:stream-support:import-downloader",
                    "Import yt-dlp",
                ),
                (
                    "shell:modal:stream-support:import-js-runtime",
                    "Import Deno",
                ),
                (
                    "shell:modal:stream-support:open-location",
                    "Open Install Location",
                ),
                ("shell:modal:stream-support:recheck", "Recheck Support"),
                ("shell:modal:stream-support:retry", "Retry URL"),
                ("shell:modal:stream-support:open-settings", "Open Settings"),
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
            "shell:modal:stream-support:install" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.install_supported
            }
            "shell:modal:stream-support:import-downloader"
            | "shell:modal:stream-support:import-js-runtime" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.integration_supported
            }
            "shell:modal:stream-support:open-location" => {
                state.stream_helper.open_install_location_available
            }
            "shell:modal:stream-support:recheck" => {
                state.pending_operation.is_none() && !state.stream_helper_remediation.active
            }
            "shell:modal:stream-support:retry" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.retry_available
            }
            "shell:modal:stream-support:open-settings" => {
                state.pending_operation.is_none() && !state.stream_helper_remediation.active
            }
            _ => true,
        }
    }

    pub(super) fn modal_close_enabled(
        state: &SyncplayGuiShellAppState,
        modal: GuiShellModal,
    ) -> bool {
        match modal {
            GuiShellModal::PlayerSetup => !state.connect_blocked_by_player_setup_issue(),
            GuiShellModal::TlsCertificatePrompt
            | GuiShellModal::UpdateNotice
            | GuiShellModal::About
            | GuiShellModal::StreamSupport => true,
        }
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

    fn display_status_value(node: &GuiWidgetNode) -> String {
        let value = node.value.as_deref().unwrap_or("(none)");
        match (node.id.as_str(), value) {
            ("main-window:connection-status", "connected") => "Connected".to_owned(),
            ("main-window:connection-status", "connecting") => "Connecting".to_owned(),
            ("main-window:connection-status", "disconnecting") => "Disconnecting".to_owned(),
            ("main-window:connection-status", "disconnected") => "Disconnected".to_owned(),
            ("main-window:connection-status", "not-configured") => "Not configured".to_owned(),
            ("main-window:playback-paused", "yes" | "true") => "Paused".to_owned(),
            ("main-window:playback-paused", "no" | "false") => "Playing".to_owned(),
            ("main-window:autoplay", "yes" | "true") => "On".to_owned(),
            ("main-window:autoplay", "no" | "false") => "Off".to_owned(),
            ("main-window:user-offset", value) => {
                Self::format_offset_seconds(value).unwrap_or_else(|| value.to_owned())
            }
            _ => value.to_owned(),
        }
    }

    fn display_status_rich_text(_ui: &egui::Ui, node: &GuiWidgetNode) -> egui::RichText {
        let text = Self::display_status_value(node);
        let palette = Self::palette();
        let color = match (node.id.as_str(), node.value.as_deref().unwrap_or("(none)")) {
            ("main-window:connection-status", "connected") => Some(palette.success_text),
            ("main-window:connection-status", "connecting" | "disconnecting") => {
                Some(palette.warning_text)
            }
            ("main-window:connection-status", "disconnected" | "not-configured") => {
                Some(palette.neutral_text)
            }
            ("main-window:playback-paused", "yes" | "true") => Some(palette.danger),
            ("main-window:playback-paused", "no" | "false") => Some(palette.success_text),
            ("main-window:autoplay", "yes" | "true") => Some(palette.success_text),
            ("main-window:autoplay", "no" | "false") => Some(palette.neutral_text),
            _ => None,
        };
        let rich_text = egui::RichText::new(text);
        if let Some(color) = color {
            rich_text.color(color).strong()
        } else {
            rich_text
        }
    }

    fn format_offset_seconds(value: &str) -> Option<String> {
        let seconds = value.parse::<f64>().ok()?;
        let sign = if seconds.is_sign_negative() { "-" } else { "" };
        let total_millis = (seconds.abs() * 1000.0).round() as u64;
        let total_seconds = total_millis / 1000;
        let millis = total_millis % 1000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        if millis > 0 {
            if hours > 0 {
                Some(format!(
                    "{sign}{hours:02}:{minutes:02}:{seconds:02}.{millis:03}"
                ))
            } else {
                Some(format!("{sign}{minutes:02}:{seconds:02}.{millis:03}"))
            }
        } else if hours > 0 {
            Some(format!("{sign}{hours:02}:{minutes:02}:{seconds:02}"))
        } else {
            Some(format!("{sign}{minutes:02}:{seconds:02}"))
        }
    }

    fn should_render_combined_status_label(node: &GuiWidgetNode) -> bool {
        node.id.starts_with("media-search:timing:")
            || node.id.starts_with("shell:command:")
            || node.id.starts_with("shell:validation:")
    }

    fn is_surface_node(node: &GuiWidgetNode) -> bool {
        matches!(node.id.as_str(), "configuration-root" | "main-window-root")
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
