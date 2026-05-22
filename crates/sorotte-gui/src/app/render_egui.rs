use eframe::egui;

use super::render_io::GuiDroppedFilesRequest;
use super::shell_state::{GuiShellAction, GuiShellModal, SorotteGuiShellAppState};
use super::widget_tree::{GuiWidgetKind, GuiWidgetNode};

mod chat;
mod controls;
mod display;
mod layout;
mod modal;
mod playback_controls;
mod playlist;
mod plugins;
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
    node_min_height_overrides: Vec<(String, f32)>,
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
pub(super) struct GuiPanelShellOptions {
    panel_width: f32,
    min_content_height: Option<f32>,
    header_height: f32,
    header_content_margin: egui::Vec2,
    body_margin: egui::Margin,
    body_horizontal_margin: f32,
}

impl GuiPanelShellOptions {
    pub(super) fn new(panel_width: f32) -> Self {
        Self {
            panel_width,
            min_content_height: None,
            header_height: GuiWidgetEguiRenderer::PANEL_HEADER_HEIGHT,
            header_content_margin: egui::vec2(10.0, 6.0),
            body_margin: egui::Margin::symmetric(10, 8),
            body_horizontal_margin: 20.0,
        }
    }

    pub(super) fn min_content_height(mut self, min_content_height: Option<f32>) -> Self {
        self.min_content_height = min_content_height;
        self
    }

    pub(super) fn header_height(mut self, header_height: f32) -> Self {
        self.header_height = header_height;
        self
    }

    pub(super) fn header_content_margin(mut self, header_content_margin: egui::Vec2) -> Self {
        self.header_content_margin = header_content_margin;
        self
    }

    pub(super) fn body_margin(mut self, body_margin: egui::Margin) -> Self {
        self.body_margin = body_margin;
        self
    }

    pub(super) fn body_horizontal_margin(mut self, body_horizontal_margin: f32) -> Self {
        self.body_horizontal_margin = body_horizontal_margin;
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct GuiSemanticPalette {
    background: egui::Color32,
    surface: egui::Color32,
    surface_muted: egui::Color32,
    border: egui::Color32,
    text: egui::Color32,
    muted_text: egui::Color32,
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
    const PANEL_RADIUS: u8 = 6;
    const PANEL_HEADER_HEIGHT: f32 = 42.0;

    fn palette_for_ui(ui: &egui::Ui) -> GuiSemanticPalette {
        Self::palette_for_dark_mode(ui.visuals().dark_mode)
    }

    fn palette_for_dark_mode(dark_mode: bool) -> GuiSemanticPalette {
        if dark_mode {
            return GuiSemanticPalette {
                background: egui::Color32::from_rgb(17, 24, 32),
                surface: egui::Color32::from_rgb(24, 34, 44),
                surface_muted: egui::Color32::from_rgb(32, 44, 55),
                border: egui::Color32::from_rgb(52, 69, 82),
                text: egui::Color32::from_rgb(232, 238, 242),
                muted_text: egui::Color32::from_rgb(154, 171, 184),
                primary: egui::Color32::from_rgb(95, 180, 194),
                primary_hover: egui::Color32::from_rgb(118, 200, 212),
                primary_text: egui::Color32::from_rgb(17, 24, 32),
                danger: egui::Color32::from_rgb(242, 139, 130),
                danger_hover: egui::Color32::from_rgb(255, 161, 151),
                danger_text: egui::Color32::from_rgb(17, 24, 32),
                success_text: egui::Color32::from_rgb(104, 211, 145),
                success_bg: egui::Color32::from_rgb(25, 57, 43),
                success_border: egui::Color32::from_rgb(70, 134, 94),
                warning_text: egui::Color32::from_rgb(246, 201, 107),
                warning_bg: egui::Color32::from_rgb(67, 51, 24),
                warning_border: egui::Color32::from_rgb(153, 118, 45),
                info_text: egui::Color32::from_rgb(95, 180, 194),
                info_bg: egui::Color32::from_rgb(24, 54, 65),
                info_border: egui::Color32::from_rgb(72, 139, 152),
                controlled_text: egui::Color32::from_rgb(184, 167, 230),
                controlled_bg: egui::Color32::from_rgb(52, 43, 76),
                controlled_border: egui::Color32::from_rgb(121, 103, 171),
                neutral_text: egui::Color32::from_rgb(232, 238, 242),
                neutral_border: egui::Color32::from_rgb(52, 69, 82),
            };
        }

        GuiSemanticPalette {
            background: egui::Color32::from_rgb(246, 248, 250),
            surface: egui::Color32::from_rgb(255, 255, 255),
            surface_muted: egui::Color32::from_rgb(238, 243, 246),
            border: egui::Color32::from_rgb(206, 216, 223),
            text: egui::Color32::from_rgb(23, 33, 43),
            muted_text: egui::Color32::from_rgb(104, 118, 131),
            primary: egui::Color32::from_rgb(47, 125, 140),
            primary_hover: egui::Color32::from_rgb(38, 105, 118),
            primary_text: egui::Color32::WHITE,
            danger: egui::Color32::from_rgb(185, 74, 72),
            danger_hover: egui::Color32::from_rgb(155, 59, 58),
            danger_text: egui::Color32::WHITE,
            success_text: egui::Color32::from_rgb(47, 133, 90),
            success_bg: egui::Color32::from_rgb(236, 248, 241),
            success_border: egui::Color32::from_rgb(151, 205, 174),
            warning_text: egui::Color32::from_rgb(183, 121, 31),
            warning_bg: egui::Color32::from_rgb(255, 248, 232),
            warning_border: egui::Color32::from_rgb(219, 181, 104),
            info_text: egui::Color32::from_rgb(47, 125, 140),
            info_bg: egui::Color32::from_rgb(237, 247, 249),
            info_border: egui::Color32::from_rgb(139, 190, 200),
            controlled_text: egui::Color32::from_rgb(107, 92, 165),
            controlled_bg: egui::Color32::from_rgb(244, 242, 251),
            controlled_border: egui::Color32::from_rgb(173, 162, 211),
            neutral_text: egui::Color32::from_rgb(23, 33, 43),
            neutral_border: egui::Color32::from_rgb(206, 216, 223),
        }
    }

    fn apply_global_style(ctx: &egui::Context) {
        let dark_mode = ctx.style().visuals.dark_mode;
        let palette = Self::palette_for_dark_mode(dark_mode);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            style.spacing.interact_size = egui::vec2(36.0, 32.0);
            style.visuals.override_text_color = Some(palette.text);
            style.visuals.weak_text_color = Some(palette.muted_text);
            style.visuals.hyperlink_color = palette.primary;
            style.visuals.faint_bg_color = palette.surface_muted;
            style.visuals.extreme_bg_color = palette.surface;
            style.visuals.text_edit_bg_color = Some(palette.surface);
            style.visuals.code_bg_color = palette.surface_muted;
            style.visuals.warn_fg_color = palette.warning_text;
            style.visuals.error_fg_color = palette.danger;
            style.visuals.window_fill = palette.surface;
            style.visuals.window_stroke = egui::Stroke::new(1.0, palette.border);
            style.visuals.window_corner_radius = egui::CornerRadius::same(6);
            style.visuals.menu_corner_radius = egui::CornerRadius::same(4);
            style.visuals.panel_fill = palette.background;
            style.visuals.selection.bg_fill = palette.primary;
            style.visuals.selection.stroke = egui::Stroke::new(1.0, palette.primary_text);
            style.visuals.widgets.noninteractive.bg_fill = palette.surface;
            style.visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, palette.border);
            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, palette.text);
            style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.inactive.bg_fill = palette.surface_muted;
            style.visuals.widgets.inactive.weak_bg_fill = palette.surface_muted;
            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, palette.border);
            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, palette.text);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
            style.visuals.widgets.hovered.bg_fill = palette.surface_muted.linear_multiply(1.05);
            style.visuals.widgets.hovered.weak_bg_fill =
                palette.surface_muted.linear_multiply(1.05);
            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, palette.primary);
            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, palette.text);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
            style.visuals.widgets.active.bg_fill = palette.primary;
            style.visuals.widgets.active.weak_bg_fill = palette.primary;
            style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, palette.primary);
            style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, palette.primary_text);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
            style.visuals.widgets.open = style.visuals.widgets.hovered;
            style.visuals.disabled_alpha = 0.42;
            style.visuals.button_frame = true;
            style.visuals.collapsing_header_frame = false;
            style.visuals.striped = true;
        });
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
        state: &SorotteGuiShellAppState,
        show_manual_pending_controls: bool,
    ) -> Vec<GuiShellAction> {
        Self::apply_global_style(ctx);
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
                ui.heading("Sorotte GUI");
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
        state: &SorotteGuiShellAppState,
    ) {
        let Some(menus) = root.find("menus-root") else {
            return;
        };
        egui::TopBottomPanel::top("sorotte-native-menu-bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for section in &menus.children {
                    ui.menu_button(&section.label, |ui| {
                        self.render_menu_section(ui, section, state);
                    });
                }
            });
        });
    }

    fn show_modal_window(&mut self, ctx: &egui::Context, state: &SorotteGuiShellAppState) {
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
        let mut panel = egui::TopBottomPanel::bottom("sorotte-native-status-bar");
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
                ui.strong("Sorotte");
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
        _state: &SorotteGuiShellAppState,
    ) {
        egui::SidePanel::left("sorotte-native-navigation")
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
        state: &SorotteGuiShellAppState,
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
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    let content_width = Self::visible_available_width(ui)
                        .min(viewport.width())
                        .max(0.0);
                    ui.set_width(content_width);
                    ui.set_max_width(content_width);
                    let surface_gutter = 12.0;
                    let surface_width = (content_width - (surface_gutter * 2.0)).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(surface_gutter);
                        ui.allocate_ui_with_layout(
                            egui::vec2(surface_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                Self::constrain_ui_width_with_clip_bleed(ui, surface_width, 3.0);
                                if let Some(active_surface) = active_surface {
                                    ui.heading(&active_surface.label);
                                    ui.add_space(10.0);
                                    self.render_node(ui, active_surface, state);
                                } else {
                                    ui.heading(&root.label);
                                    ui.label("No active surface is currently selected.");
                                }
                            },
                        );
                    });
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
        } else if node_id == "plugins-root" {
            let body = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 6.0, rect.top() + 6.0),
                egui::pos2(rect.right() - 6.0, rect.bottom() - 4.0),
            );
            painter.rect_stroke(body, 3, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(body.left() + 4.0, rect.top() + 1.0),
                    egui::pos2(body.left() + 4.0, body.top()),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(body.right() - 4.0, rect.top() + 1.0),
                    egui::pos2(body.right() - 4.0, body.top()),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(body.center().x, body.bottom()),
                    egui::pos2(body.center().x, rect.bottom()),
                ],
                stroke,
            );
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
        state: &SorotteGuiShellAppState,
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
        state: &SorotteGuiShellAppState,
    ) {
        if node.id == "main-window:browser" {
            self.render_room_browser(ui, node, state);
            return;
        }
        if node.id == "main-window:connection" {
            self.render_combined_room_panel(ui, node, state);
            return;
        }
        if node.id == "main-window:top-region" {
            self.render_room_dashboard(ui, node, state);
            return;
        }
        if node.id == "plugins-root" {
            self.render_plugins_surface(ui, node, state);
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
        if node.id == "main-window:playlist-surface" {
            self.render_playlist_surface_panel(ui, node, state);
            return;
        }
        if node.id == "main-window:playlist-playback" {
            self.render_playlist_playback_footer(ui, node, state, 76.0);
            return;
        }
        if node.id == "main-window:playlist-edit" || node.id == "main-window:playlist-url-edit" {
            self.render_inline_editor_panel(ui, node, state);
            return;
        }
        if node.id == "config-commands" {
            self.render_setup_command_bar(ui, node, state);
            return;
        }
        if node.id == "configuration:action-alert" || node.id == "plugins:stream-support:alert" {
            self.render_action_alert_panel(ui, node, state);
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
                let list_width = Self::visible_available_width(ui);
                let response = egui::Frame::new()
                    .fill(Self::palette_for_ui(ui).surface)
                    .stroke(egui::Stroke::new(1.0, Self::palette_for_ui(ui).border))
                    .corner_radius(egui::CornerRadius::same(Self::PANEL_RADIUS))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        let content_width = Self::width_inside_horizontal_margin(list_width, 20.0);
                        ui.set_width(content_width);
                        ui.set_max_width(content_width);
                        if let Some(min_content_height) = self.node_min_content_height(node) {
                            ui.set_min_height(min_content_height);
                        }
                        ui.label(
                            egui::RichText::new(&node.label)
                                .strong()
                                .color(Self::palette_for_ui(ui).neutral_text),
                        );
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
        state: &SorotteGuiShellAppState,
    ) {
        let panel_width = Self::panel_available_width(ui);
        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width)
                .min_content_height(self.node_min_content_height(node)),
            |renderer, ui, _body_width| {
                for child in node.children.iter().filter(|child| {
                    !(child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"))
                }) {
                    renderer.render_node(ui, child, state);
                }
            },
        );
    }

    pub(super) fn render_panel_shell(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        options: GuiPanelShellOptions,
        add_body: impl FnOnce(&mut Self, &mut egui::Ui, f32),
    ) {
        let close_button = node
            .children
            .iter()
            .find(|child| child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"));
        self.render_panel_shell_with_header(
            ui,
            options,
            |renderer, ui, header_width| {
                renderer.render_panel_header_content(ui, node, close_button, state, header_width);
            },
            add_body,
        );
    }

    pub(super) fn render_panel_shell_with_header(
        &mut self,
        ui: &mut egui::Ui,
        options: GuiPanelShellOptions,
        add_header: impl FnOnce(&mut Self, &mut egui::Ui, f32),
        add_body: impl FnOnce(&mut Self, &mut egui::Ui, f32),
    ) {
        let panel_width = options.panel_width.max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(panel_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(panel_width);
                ui.set_max_width(panel_width);
                let panel_left = ui.cursor().left();
                let panel_clip_rect = egui::Rect::from_min_max(
                    egui::pos2(panel_left, ui.clip_rect().top()),
                    egui::pos2(panel_left + panel_width, ui.clip_rect().bottom()),
                );
                ui.shrink_clip_rect(panel_clip_rect);
                let panel_response = egui::Frame::new()
                    .fill(Self::palette_for_ui(ui).surface)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(Self::PANEL_RADIUS))
                    .inner_margin(egui::Margin::same(0))
                    .show(ui, |ui| {
                        ui.set_width(panel_width);
                        ui.set_max_width(panel_width);
                        if let Some(min_content_height) = options.min_content_height {
                            ui.set_min_height(min_content_height);
                        }

                        let (header_rect, _) = ui.allocate_exact_size(
                            egui::vec2(panel_width, options.header_height),
                            egui::Sense::hover(),
                        );
                        Self::paint_panel_header_background(ui, header_rect);
                        let content_rect = header_rect.shrink2(options.header_content_margin);
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(content_rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                            |ui| {
                                ui.set_width(content_rect.width());
                                ui.set_max_width(content_rect.width());
                                add_header(self, ui, content_rect.width());
                            },
                        );
                        Self::paint_panel_header_separator(ui, header_rect);

                        egui::Frame::new()
                            .inner_margin(options.body_margin)
                            .show(ui, |ui| {
                                let body_width = Self::width_inside_horizontal_margin(
                                    panel_width,
                                    options.body_horizontal_margin,
                                );
                                ui.set_width(body_width);
                                ui.set_max_width(body_width);
                                add_body(self, ui, body_width);
                            });
                    });
                let visual_rect = egui::Rect::from_min_max(
                    egui::pos2(panel_left, panel_response.response.rect.top()),
                    egui::pos2(
                        panel_left + panel_width,
                        panel_response.response.rect.bottom(),
                    ),
                );
                Self::paint_visible_panel_outline(ui, visual_rect, Self::PANEL_RADIUS);
            },
        );
    }

    fn render_inline_editor_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        if node.id == "main-window:playlist-url-edit" {
            self.render_playlist_url_inline_editor_panel(ui, node, state);
            return;
        }
        let close_button = node
            .children
            .iter()
            .find(|child| child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"));
        let palette = Self::palette_for_ui(ui);
        egui::Frame::new()
            .fill(palette.surface_muted)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(egui::RichText::new(&node.label).color(palette.neutral_text));
                    if let Some(close_button) = close_button {
                        ui.add_space(8.0);
                        let response = ui.add_enabled(
                            close_button.enabled,
                            egui::Button::new(
                                egui::RichText::new("X").strong().color(palette.danger),
                            )
                            .frame(false)
                            .min_size(egui::vec2(32.0, 32.0)),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                response.enabled(),
                                &close_button.label,
                            )
                        });
                        if Self::attach_hover_text(response, close_button.label.clone()).clicked() {
                            self.handle_button_node_click(state, close_button);
                        }
                    }
                });
                ui.add_space(4.0);
                for child in node.children.iter().filter(|child| {
                    !(child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"))
                }) {
                    self.render_node(ui, child, state);
                }
            });
    }

    fn render_playlist_url_inline_editor_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let close_button = node
            .children
            .iter()
            .find(|child| child.kind == GuiWidgetKind::Button && child.id.ends_with(":close"));
        let text_node = Self::find_descendant_by_id(node, "main-window:playlist-url-edit:text");
        let helper_node = Self::find_descendant_by_id(node, "main-window:playlist-url-edit:helper");
        let commit_node = Self::find_descendant_by_id(node, "main-window:playlist-url-edit:commit");
        let palette = Self::palette_for_ui(ui);

        egui::Frame::new()
            .fill(palette.surface_muted)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(egui::RichText::new(&node.label).color(palette.neutral_text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(close_button) = close_button {
                            let response = ui.add_enabled(
                                close_button.enabled,
                                egui::Button::new(
                                    egui::RichText::new("X").strong().color(palette.danger),
                                )
                                .frame(false)
                                .min_size(egui::vec2(32.0, 28.0)),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    response.enabled(),
                                    &close_button.label,
                                )
                            });
                            if Self::attach_hover_text(response, close_button.label.clone())
                                .clicked()
                            {
                                self.handle_button_node_click(state, close_button);
                            }
                        }
                    });
                });

                if let Some(text_node) = text_node {
                    ui.add_space(4.0);
                    let mut value = text_node.value.clone().unwrap_or_default();
                    let response = ui.add_enabled(
                        text_node.enabled,
                        egui::TextEdit::multiline(&mut value)
                            .desired_width(Self::visible_available_width(ui).max(1.0))
                            .desired_rows(3)
                            .hint_text("https://..."),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::TextEdit,
                            response.enabled(),
                            text_node.label.clone(),
                        )
                    });
                    if let Some(actions) = Self::actions_for_text_input_node(
                        state,
                        text_node,
                        &value,
                        response.changed(),
                        false,
                    ) {
                        self.actions.extend(actions);
                    }
                }

                ui.add_space(6.0);
                ui.horizontal_top(|ui| {
                    let available_width = Self::visible_available_width(ui);
                    let button_width = available_width.clamp(160.0, 240.0);
                    let helper_width = (available_width - button_width - 8.0).max(0.0);
                    if let Some(helper_node) = helper_node {
                        ui.allocate_ui_with_layout(
                            egui::vec2(helper_width, ui.spacing().interact_size.y),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let helper_text = helper_node.value.as_deref().unwrap_or("");
                                ui.label(
                                    egui::RichText::new(helper_text)
                                        .small()
                                        .color(palette.muted_text),
                                );
                            },
                        );
                    }
                    if let Some(commit_node) = commit_node {
                        ui.add_space(8.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(button_width, ui.spacing().interact_size.y),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(button_width);
                                self.render_button_like(ui, commit_node, state);
                            },
                        );
                    }
                });
            });
    }

    fn find_descendant_by_id<'a>(node: &'a GuiWidgetNode, id: &str) -> Option<&'a GuiWidgetNode> {
        if node.id == id {
            return Some(node);
        }
        node.children
            .iter()
            .find_map(|child| Self::find_descendant_by_id(child, id))
    }

    fn render_action_alert_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let level = node
            .children
            .iter()
            .find(|child| child.id.ends_with(":level"))
            .and_then(|child| child.value.as_deref())
            .unwrap_or(node.label.as_str());
        let message = node
            .children
            .iter()
            .find(|child| child.id.ends_with(":message"))
            .and_then(|child| child.value.as_deref())
            .unwrap_or("");
        let (fill, stroke, text_color) = Self::alert_colors_for_level(ui, level);
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(level.to_ascii_uppercase())
                                .small()
                                .strong()
                                .color(text_color),
                        );
                        ui.label(egui::RichText::new(message).color(text_color));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if let Some(close_button) = node
                            .children
                            .iter()
                            .find(|child| child.id.ends_with(":close"))
                        {
                            self.render_panel_close_button(ui, close_button, state);
                        }
                    });
                });
                for child in node.children.iter().filter(|child| {
                    child.id.ends_with(":actions")
                        || (child.kind == GuiWidgetKind::Button && !child.id.ends_with(":close"))
                }) {
                    ui.add_space(6.0);
                    self.render_node(ui, child, state);
                }
            });
    }

    fn alert_colors_for_level(
        ui: &egui::Ui,
        level: &str,
    ) -> (egui::Color32, egui::Color32, egui::Color32) {
        let palette = Self::palette_for_ui(ui);
        match level {
            "success" => (
                palette.success_bg,
                palette.success_border,
                palette.success_text,
            ),
            "warning" => (
                palette.warning_bg,
                palette.warning_border,
                palette.warning_text,
            ),
            "error" => (
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(72, 37, 35)
                } else {
                    egui::Color32::from_rgb(255, 240, 239)
                },
                palette.danger,
                palette.danger,
            ),
            _ => (palette.info_bg, palette.info_border, palette.info_text),
        }
    }

    fn render_panel_header_content(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        close_button: Option<&GuiWidgetNode>,
        state: &SorotteGuiShellAppState,
        content_width: f32,
    ) {
        let palette = Self::palette_for_ui(ui);
        ui.horizontal(|ui| {
            ui.set_width(content_width);
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
    }

    fn paint_panel_header_background(ui: &egui::Ui, rect: egui::Rect) {
        ui.painter().rect_filled(
            rect,
            Self::panel_header_corner_radius(),
            Self::panel_header_fill(ui),
        );
    }

    fn panel_header_fill(ui: &egui::Ui) -> egui::Color32 {
        Self::palette_for_ui(ui).surface_muted
    }

    fn panel_header_stroke(ui: &egui::Ui) -> egui::Stroke {
        egui::Stroke::new(1.0, Self::palette_for_ui(ui).border)
    }

    pub(super) fn panel_header_corner_radius() -> egui::CornerRadius {
        egui::CornerRadius {
            nw: Self::PANEL_RADIUS,
            ne: Self::PANEL_RADIUS,
            sw: 0,
            se: 0,
        }
    }

    pub(super) fn paint_panel_header_separator(ui: &egui::Ui, rect: egui::Rect) {
        let visible_rect = rect.intersect(ui.clip_rect());
        if visible_rect.width() <= 1.0 {
            return;
        }
        let y = visible_rect.bottom();
        let edge_inset = 0.5;
        let left = visible_rect.left() + edge_inset;
        let right = visible_rect.right() - edge_inset;
        if right <= left {
            return;
        }
        ui.painter().line_segment(
            [egui::pos2(left, y), egui::pos2(right, y)],
            Self::panel_header_stroke(ui),
        );
    }

    pub(super) fn paint_visible_panel_outline(ui: &egui::Ui, rect: egui::Rect, corner_radius: u8) {
        let visible_rect = rect.intersect(ui.clip_rect());
        if visible_rect.width() <= 1.0 || visible_rect.height() <= 1.0 {
            return;
        }
        ui.painter().rect_stroke(
            visible_rect.shrink2(egui::vec2(0.5, 0.5)),
            corner_radius,
            egui::Stroke::new(1.0, Self::palette_for_ui(ui).border),
            egui::StrokeKind::Inside,
        );
    }

    pub(super) fn node_min_content_height(&self, node: &GuiWidgetNode) -> Option<f32> {
        let override_height = self
            .node_min_height_overrides
            .iter()
            .rev()
            .find(|(id, _)| id == &node.id)
            .map(|(_, height)| *height);
        match (node.min_content_height, override_height) {
            (Some(node_height), Some(override_height)) => Some(node_height.max(override_height)),
            (Some(node_height), None) => Some(node_height),
            (None, Some(override_height)) => Some(override_height),
            (None, None) => None,
        }
    }

    pub(super) fn push_node_min_height_override(&mut self, node_id: &str, min_height: f32) {
        self.node_min_height_overrides
            .push((node_id.to_owned(), min_height.max(0.0)));
    }

    pub(super) fn pop_node_min_height_override(&mut self) {
        self.node_min_height_overrides.pop();
    }

    pub(super) fn room_dashboard_layout_for_width(width: f32) -> GuiRoomDashboardLayout {
        let width = width.max(0.0);
        if width < 680.0 {
            GuiRoomDashboardLayout::Narrow
        } else if width < 860.0 {
            GuiRoomDashboardLayout::Medium
        } else {
            GuiRoomDashboardLayout::Wide
        }
    }

    fn render_panel_close_button(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
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
