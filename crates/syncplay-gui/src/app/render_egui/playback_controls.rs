use std::time::Duration;

use eframe::egui;

use super::super::shell_state::SyncplayGuiShellAppState;
use super::super::widget_tree::GuiWidgetNode;
use super::GuiWidgetEguiRenderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiPlaybackControlIcon {
    Play,
    Pause,
    TogglePause,
    Seek,
    UndoSeek,
    SetOffset,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn render_playback_icon_button(
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

    pub(super) fn render_playback_ready_button(
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
                    let available_width = Self::visible_available_width(ui);
                    let button_width = Self::playback_ready_button_width(available_width);
                    let side_space = ((available_width - button_width).max(0.0)) * 0.5;
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

    pub(super) fn playback_control_icon(node: &GuiWidgetNode) -> Option<GuiPlaybackControlIcon> {
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
        let available_height = ui.available_height();
        let min_height = ui.spacing().interact_size.y.clamp(1.0, 36.0);
        let height = if available_height.is_finite() && available_height > 0.0 {
            available_height.clamp(min_height, 40.0)
        } else {
            40.0
        };
        egui::vec2(Self::visible_available_width(ui).max(1.0), height)
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
        let palette = Self::palette_for_ui(ui);
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
}
