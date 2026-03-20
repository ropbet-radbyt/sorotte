use eframe::egui;

use super::render_io::GuiDroppedFilesRequest;
use super::shell_state::{GuiShellAction, GuiShellModal, SyncplayGuiShellAppState};
use super::widget_tree::{GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer};

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
            .default_width(240.0)
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
            GuiWidgetKind::Panel => {
                egui::Frame::group(ui.style()).show(ui, |ui| {
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

    fn render_leaf(
        &mut self,
        ui: &mut egui::Ui,
        node: &GuiWidgetNode,
        state: &SyncplayGuiShellAppState,
    ) {
        match node.kind {
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
                if ui
                    .add_enabled(node.enabled, egui::Button::new(Self::display_text(node)))
                    .clicked()
                {
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
                    .desired_width(260.0),
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
                    .desired_width(360.0)
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
                    .width(260.0)
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
