use eframe::egui;

use super::super::shell_state::SorotteGuiShellAppState;
use super::super::widget_tree::GuiWidgetNode;
use super::GuiWidgetEguiRenderer;

impl GuiWidgetEguiRenderer {
    pub(super) fn render_chat_history(&mut self, ui: &mut egui::Ui, node: &GuiWidgetNode) {
        let outer_width = Self::visible_available_width(ui);
        let content_width = Self::width_inside_horizontal_margin(outer_width, 2.0);
        let palette = Self::palette_for_ui(ui);
        egui::Frame::new()
            .fill(palette.surface)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius::same(Self::PANEL_RADIUS))
            .inner_margin(egui::Margin::same(0))
            .show(ui, |ui| {
                let history_height = node.min_content_height.unwrap_or(180.0).clamp(120.0, 260.0);
                ui.set_min_width(content_width);
                ui.set_max_width(content_width);
                ui.set_min_height(history_height);
                ui.set_max_height(history_height);
                egui::ScrollArea::vertical()
                    .id_salt(&node.id)
                    .max_height(history_height)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(content_width);
                        ui.set_max_width(content_width);
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
        let available_width = Self::visible_available_width(ui);
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
        let available_width = Self::visible_available_width(ui);
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

    pub(super) fn render_chat_compose(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
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
            let input_width = (Self::visible_available_width(ui) - send_width - 8.0).max(1.0);
            let response = ui.add_enabled(
                input_node.enabled,
                egui::TextEdit::singleline(&mut value)
                    .id_salt(&input_node.id)
                    .return_key(None)
                    .desired_width(input_width)
                    .hint_text("Type a message..."),
            );
            let response = Self::attach_node_tooltip(response, input_node);
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
}
