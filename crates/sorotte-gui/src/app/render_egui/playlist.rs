use eframe::egui;

use super::super::shell_state::{GuiShellAction, SorotteGuiShellAppState};
use super::super::widget_tree::GuiWidgetNode;
use super::{GuiPanelShellOptions, GuiWidgetEguiRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuiDraggedPlaylistRow {
    index: usize,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn render_playlist_header_actions(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
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

    pub(super) fn render_playlist_surface_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let panel_width = Self::panel_available_width(ui);
        let min_height = self.node_min_content_height(node).unwrap_or(0.0);
        let footer_height = 76.0;
        self.render_panel_shell(
            ui,
            node,
            state,
            GuiPanelShellOptions::new(panel_width).min_content_height(Some(min_height)),
            |renderer, ui, _body_width| {
                let body_top = ui.cursor().top();
                let editor_active = node.children.iter().any(|child| {
                    child.id == "main-window:playlist-edit"
                        || child.id == "main-window:playlist-url-edit"
                });
                for child in node
                    .children
                    .iter()
                    .filter(|child| child.id != "main-window:playlist-playback")
                {
                    renderer.render_node(ui, child, state);
                }
                let used_height = ui.cursor().top() - body_top;
                let target_body_height =
                    (min_height - Self::PANEL_HEADER_HEIGHT - 16.0).max(footer_height);
                let spacer = if editor_active {
                    8.0
                } else {
                    (target_body_height - used_height - footer_height).max(0.0)
                };
                if spacer > 0.0 {
                    ui.add_space(spacer);
                }
                if let Some(footer) = node
                    .children
                    .iter()
                    .find(|child| child.id == "main-window:playlist-playback")
                {
                    renderer.render_playlist_playback_footer(ui, footer, state, footer_height);
                }
            },
        );
    }

    pub(super) fn render_playlist_playback_footer(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
        height: f32,
    ) {
        let width = Self::visible_available_width(ui);
        let content_width = Self::width_inside_horizontal_margin(width, 24.0);
        let palette = Self::palette_for_ui(ui);
        let actions_node = node
            .children
            .iter()
            .find(|child| child.id == "main-window:controls:playback-actions");
        egui::Frame::new()
            .fill(palette.surface_muted)
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: 6,
                se: 6,
            })
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                let content_height = (height - 20.0).max(40.0);
                ui.set_min_height(content_height);
                ui.set_width(content_width);
                ui.set_max_width(content_width);
                ui.horizontal(|ui| {
                    if let Some(actions_node) = actions_node {
                        let controls_width = Self::playback_controls_width(actions_node);
                        let side_space =
                            ((Self::visible_available_width(ui) - controls_width) * 0.5).max(0.0);
                        if side_space > 0.0 {
                            ui.add_space(side_space);
                        }
                        ui.allocate_ui_with_layout(
                            egui::vec2(controls_width, 40.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(controls_width);
                                ui.set_max_width(controls_width);
                                self.render_layout(ui, actions_node, state);
                            },
                        );
                    }
                });
            });
    }

    fn playback_controls_width(actions: &GuiWidgetNode) -> f32 {
        let button_count = actions.children.len() as f32;
        if button_count <= 0.0 {
            0.0
        } else {
            (button_count * 40.0) + ((button_count - 1.0) * 8.0)
        }
    }

    pub(super) fn render_playback_state_icon(ui: &mut egui::Ui, node: &GuiWidgetNode, size: f32) {
        let paused = matches!(node.value.as_deref(), Some("yes" | "true"));
        let palette = Self::palette_for_ui(ui);
        let label = if paused {
            "Room state: paused"
        } else {
            "Room state: playing"
        };
        let (fill, stroke, icon_color) = if paused {
            (
                palette.warning_bg,
                palette.warning_border,
                palette.warning_text,
            )
        } else {
            (
                palette.success_bg,
                palette.success_border,
                palette.success_text,
            )
        };
        let size = size.max(24.0);
        let (_, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, response.enabled(), label)
        });
        let response = Self::attach_hover_text(response, label);
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            (size * 0.5) as u8,
            fill,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        let icon_rect = rect.shrink2(egui::vec2(size * 0.3, size * 0.25));
        if paused {
            let bar_width = icon_rect.width() * 0.26;
            let gap = icon_rect.width() * 0.18;
            let left = icon_rect.center().x - ((bar_width * 2.0) + gap) / 2.0;
            let first_bar = egui::Rect::from_min_max(
                egui::pos2(left, icon_rect.top()),
                egui::pos2(left + bar_width, icon_rect.bottom()),
            );
            let second_bar = first_bar.translate(egui::vec2(bar_width + gap, 0.0));
            ui.painter().rect_filled(first_bar, 1.5, icon_color);
            ui.painter().rect_filled(second_bar, 1.5, icon_color);
        } else {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(icon_rect.left(), icon_rect.top()),
                    egui::pos2(icon_rect.right(), icon_rect.center().y),
                    egui::pos2(icon_rect.left(), icon_rect.bottom()),
                ],
                icon_color,
                egui::Stroke::NONE,
            ));
        }
    }

    pub(super) fn render_playlist_list(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
    ) {
        let playlist_focus_id = Self::playlist_focus_id();
        let playlist_focused = ui.ctx().memory(|mem| mem.has_focus(playlist_focus_id));
        let playlist_len = node.children.len();
        let mut first_row_rect = None;
        let mut last_row_rect = None;
        let mut playlist_focus_requested = false;
        let available_width = Self::visible_available_width(ui);
        let min_height = self.node_min_content_height(node).unwrap_or(0.0);
        let response = ui.allocate_ui_with_layout(
            egui::vec2(available_width, min_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(available_width);
                ui.set_max_width(available_width);
                if let Some(min_content_height) = self.node_min_content_height(node) {
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
            },
        );
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
        let available_width = Self::visible_available_width(ui);
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

    fn render_playlist_list_item(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SorotteGuiShellAppState,
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
        let remove_button = node
            .children
            .iter()
            .find(|child| child.id.ends_with(":remove"));
        let text = Self::display_text(node).to_owned();
        let is_room_active = state.main_window.active_playlist_index == Some(index);

        let (button_response, remove_clicked) = ui
            .push_id(&node.id, |ui| {
                ui.add_enabled_ui(node.enabled, |ui| {
                    let width = Self::visible_available_width(ui);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, 48.0), egui::Sense::hover());
                    let remove_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 18.0, rect.center().y),
                        egui::vec2(30.0, 30.0),
                    );
                    let row_rect = egui::Rect::from_min_max(
                        rect.left_top(),
                        egui::pos2(remove_rect.left() - 8.0, rect.bottom()),
                    );
                    let response = ui.interact(
                        row_rect,
                        ui.id().with("row"),
                        Self::playlist_row_sense(can_drag_reorder),
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
                        rect,
                        &response,
                        &text,
                        node.selected,
                        is_room_active,
                        remove_button.is_some(),
                    );
                    let response = if truncated {
                        response.on_hover_text(text.clone())
                    } else {
                        response
                    };
                    let remove_clicked = remove_button.is_some_and(|button| {
                        let remove_response =
                            ui.interact(remove_rect, ui.id().with("remove"), egui::Sense::click());
                        remove_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                remove_response.enabled(),
                                button.label.clone(),
                            )
                        });
                        let remove_response =
                            Self::attach_hover_text(remove_response, button.label.clone());
                        Self::paint_playlist_remove_button(ui, &remove_response, button.enabled);
                        remove_response.clicked() && button.enabled
                    });
                    (response, remove_clicked)
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

        let pointer_actions = if remove_clicked {
            vec![
                GuiShellAction::SelectMainWindowPlaylist(index),
                GuiShellAction::RemoveSelectedMainWindowPlaylist,
            ]
        } else {
            Self::playlist_row_pointer_actions(
                index,
                button_response.clicked(),
                button_response.double_clicked(),
            )
        };
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

    pub(super) fn playlist_row_pointer_actions(
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

    pub(super) fn playlist_focus_sense() -> egui::Sense {
        egui::Sense::focusable_noninteractive()
    }

    pub(super) fn playlist_row_sense(can_drag_reorder: bool) -> egui::Sense {
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
        state: &SorotteGuiShellAppState,
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

    fn paint_playlist_row(
        ui: &egui::Ui,
        row_rect: egui::Rect,
        response: &egui::Response,
        label: &str,
        is_selected: bool,
        is_room_active: bool,
        has_remove_button: bool,
    ) -> bool {
        let visuals = ui.style().interact(response);
        let palette = Self::palette_for_ui(ui);
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
        let rect = row_rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            5,
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
        let remove_padding = if has_remove_button { 46.0 } else { 12.0 };
        let text_right = (rect.right() - remove_padding).max(text_left);
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

    fn paint_playlist_remove_button(ui: &egui::Ui, response: &egui::Response, enabled: bool) {
        let palette = Self::palette_for_ui(ui);
        let fill = if response.hovered() && enabled {
            palette.danger.gamma_multiply(0.14)
        } else {
            palette.surface
        };
        let stroke_color = if response.hovered() && enabled {
            palette.danger
        } else {
            palette.border
        };
        let icon_color = if enabled {
            palette.danger
        } else {
            ui.visuals().weak_text_color()
        };
        let rect = response.rect.shrink2(egui::vec2(0.5, 0.5));
        ui.painter().rect(
            rect,
            4,
            fill,
            egui::Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Inside,
        );
        let stroke = egui::Stroke::new(1.8, icon_color);
        let icon_rect = rect.shrink2(egui::vec2(9.5, 9.5));
        ui.painter()
            .line_segment([icon_rect.left_top(), icon_rect.right_bottom()], stroke);
        ui.painter()
            .line_segment([icon_rect.right_top(), icon_rect.left_bottom()], stroke);
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
}
