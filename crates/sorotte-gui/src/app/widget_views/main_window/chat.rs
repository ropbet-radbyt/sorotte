use super::*;

impl SorotteGuiShellAppState {
    pub(in crate::app) fn main_window_chat_panel(&self) -> GuiWidgetNode {
        let chat_unavailable_reason =
            (!self.commands.can_send_chat_message).then(|| self.chat_send_unavailable_reason());
        let chat_input_node = GuiWidgetNode::leaf(
            "main-window:chat-input",
            "Chat Input",
            GuiWidgetKind::TextInput,
            Some(self.outgoing_chat_message.clone().unwrap_or_default()),
            self.commands.can_send_chat_message,
            false,
        );
        let chat_input_node = if let Some(reason) = chat_unavailable_reason.as_ref() {
            chat_input_node.with_tooltip(reason.clone())
        } else {
            chat_input_node
        };

        let send_node = GuiWidgetNode::leaf(
            "main-window:chat:send",
            "Send",
            GuiWidgetKind::Button,
            None,
            self.commands.can_send_chat_message
                && self
                    .outgoing_chat_message
                    .as_deref()
                    .and_then(normalized_editable_text)
                    .is_some(),
            false,
        );
        let send_node = if let Some(reason) = chat_unavailable_reason {
            send_node.with_tooltip(reason)
        } else {
            send_node
        };

        GuiWidgetNode::branch(
            "main-window:chat-panel",
            "Chat",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::branch(
                    "main-window:chat",
                    "Chat History",
                    GuiWidgetKind::List,
                    self.main_window
                        .chat
                        .iter()
                        .enumerate()
                        .map(|(index, row)| {
                            GuiWidgetNode::leaf(
                                format!("main-window:chat:{index}"),
                                &row.sender,
                                GuiWidgetKind::ListItem,
                                Some(row.message.clone()),
                                true,
                                false,
                            )
                        })
                        .collect(),
                )
                .with_min_content_height(180.0),
                GuiWidgetNode::layout(
                    "main-window:chat-compose",
                    "Chat Compose",
                    GuiLayoutMode::Stack,
                    vec![chat_input_node, send_node],
                ),
            ],
        )
    }
}
