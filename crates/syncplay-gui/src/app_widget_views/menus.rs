use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn menu_dialog_widget_tree(&self) -> GuiWidgetNode {
        let mut dialog_children = vec![
            GuiWidgetNode::leaf(
                "menus:dialog:tls",
                "TLS Certificate Prompt",
                GuiWidgetKind::Status,
                Some(bool_label(self.menus.tls_prompt_expected).to_owned()),
                true,
                self.open_modal == Some(GuiShellModal::TlsCertificatePrompt),
            ),
            GuiWidgetNode::leaf(
                "menus:dialog:update",
                "Update Notice",
                GuiWidgetKind::Status,
                Some(bool_label(self.menus.update_notice_expected).to_owned()),
                true,
                self.open_modal == Some(GuiShellModal::UpdateNotice),
            ),
            GuiWidgetNode::leaf(
                "menus:dialog:about",
                "About Dialog",
                GuiWidgetKind::Status,
                Some(bool_label(self.menus.about_dialog_available).to_owned()),
                self.menus.about_dialog_available,
                self.open_modal == Some(GuiShellModal::About),
            ),
            GuiWidgetNode::leaf(
                "menus:dialog:player-setup",
                "mpv Setup",
                GuiWidgetKind::Status,
                Some(bool_label(self.player_setup_issue.is_some()).to_owned()),
                self.player_setup_issue.is_some(),
                self.open_modal == Some(GuiShellModal::PlayerSetup),
            ),
            GuiWidgetNode::leaf(
                "menus:dialog:stream-support",
                "Stream Support",
                GuiWidgetKind::Status,
                Some(
                    bool_label(self.stream_helper.health != GuiStreamHelperHealth::Healthy)
                        .to_owned(),
                ),
                self.stream_helper.health != GuiStreamHelperHealth::Healthy,
                self.open_modal == Some(GuiShellModal::StreamSupport),
            ),
        ];
        if let Some(message) = self.update_check.message.as_ref() {
            dialog_children.push(GuiWidgetNode::leaf(
                "menus:update:message",
                "Update Message",
                GuiWidgetKind::Status,
                Some(message.clone()),
                true,
                false,
            ));
        }
        if let Some(url) = self.update_check.url.as_ref() {
            dialog_children.push(GuiWidgetNode::leaf(
                "menus:update:url",
                "Update URL",
                GuiWidgetKind::Status,
                Some(url.clone()),
                true,
                false,
            ));
        }
        dialog_children.push(GuiWidgetNode::leaf(
            "menus:about:summary",
            "About Syncplay",
            GuiWidgetKind::Status,
            Some("Rust native GUI shell for Syncplay.".to_owned()),
            self.menus.about_dialog_available,
            false,
        ));
        dialog_children.push(GuiWidgetNode::leaf(
            "menus:about:details",
            "About Details",
            GuiWidgetKind::Status,
            Some(
                "Use Help and Check for Updates from this surface; only TLS opens a modal."
                    .to_owned(),
            ),
            self.menus.about_dialog_available,
            false,
        ));

        let mut children = self
            .menus
            .sections
            .iter()
            .enumerate()
            .map(|(section_index, section)| {
                GuiWidgetNode::branch(
                    format!("menus:section:{section_index}"),
                    section.title,
                    GuiWidgetKind::Panel,
                    section
                        .actions
                        .iter()
                        .enumerate()
                        .map(|(action_index, action)| {
                            GuiWidgetNode::leaf(
                                format!("menus:action:{section_index}:{action_index}"),
                                action.label,
                                GuiWidgetKind::Button,
                                None,
                                action.enabled,
                                action.is_selected,
                            )
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>();

        children.push(GuiWidgetNode::branch(
            "menus:dialogs",
            "Dialogs",
            GuiWidgetKind::Panel,
            dialog_children,
        ));

        GuiWidgetNode::branch(
            "menus-root",
            "Menus & Dialogs",
            GuiWidgetKind::Panel,
            children,
        )
    }

    pub(crate) fn shell_modal_widget_tree(&self) -> GuiWidgetNode {
        let Some(modal) = self.open_modal else {
            return GuiWidgetNode::branch("shell:modal", "Modal", GuiWidgetKind::Panel, Vec::new());
        };
        let mut children = vec![GuiWidgetNode::leaf(
            "shell:modal:kind",
            "Modal Kind",
            GuiWidgetKind::Status,
            Some(modal.label().to_owned()),
            true,
            false,
        )];
        match modal {
            GuiShellModal::UpdateNotice => {
                if let Some(message) = self.update_check.message.as_ref() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:update:message",
                        "Message",
                        GuiWidgetKind::Status,
                        Some(message.clone()),
                        true,
                        false,
                    ));
                }
                if let Some(url) = self.update_check.url.as_ref() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:update:url",
                        "Update URL",
                        GuiWidgetKind::Status,
                        Some(url.clone()),
                        true,
                        false,
                    ));
                }
            }
            GuiShellModal::PlayerSetup => {
                children.push(GuiWidgetNode::leaf(
                    "shell:modal:player-setup:summary",
                    "Summary",
                    GuiWidgetKind::Status,
                    self.player_setup_issue_summary().map(str::to_owned),
                    true,
                    false,
                ));
                if let Some(issue) = self.player_setup_issue.as_ref() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:player-setup:detail",
                        "Detail",
                        GuiWidgetKind::Status,
                        Some(issue.message.clone()),
                        true,
                        false,
                    ));
                }
                if self.connect_blocked_by_player_setup_issue() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:player-setup:blocking",
                        "Connect Status",
                        GuiWidgetKind::Status,
                        self.player_setup_connect_block_message(),
                        true,
                        false,
                    ));
                }
            }
            GuiShellModal::StreamSupport => {
                children.push(GuiWidgetNode::leaf(
                    "shell:modal:stream-support:summary",
                    "Summary",
                    GuiWidgetKind::Status,
                    self.stream_helper_issue_summary().map(str::to_owned),
                    true,
                    false,
                ));
                children.push(GuiWidgetNode::leaf(
                    "shell:modal:stream-support:health",
                    "Health",
                    GuiWidgetKind::Status,
                    Some(self.stream_helper.health.label().to_owned()),
                    true,
                    false,
                ));
                if let Some(target) = self.stream_helper.target.as_ref() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:stream-support:target",
                        "Target",
                        GuiWidgetKind::Status,
                        Some(target.clone()),
                        true,
                        false,
                    ));
                }
                if let Some(message) = self.stream_helper.message.as_ref() {
                    children.push(GuiWidgetNode::leaf(
                        "shell:modal:stream-support:detail",
                        "Detail",
                        GuiWidgetKind::Status,
                        Some(message.clone()),
                        true,
                        false,
                    ));
                }
            }
            _ => {}
        }
        children.extend(GuiWidgetEguiRenderer::modal_actions(modal).into_iter().map(
            |(id, label)| {
                GuiWidgetNode::leaf(
                    id,
                    label,
                    GuiWidgetKind::Button,
                    None,
                    GuiWidgetEguiRenderer::modal_action_enabled(self, id),
                    false,
                )
            },
        ));
        children.push(GuiWidgetNode::leaf(
            "shell:modal:close",
            "Close",
            GuiWidgetKind::Button,
            None,
            GuiWidgetEguiRenderer::modal_close_enabled(self, modal),
            false,
        ));
        GuiWidgetNode::branch("shell:modal", "Modal", GuiWidgetKind::Panel, children)
    }
}
