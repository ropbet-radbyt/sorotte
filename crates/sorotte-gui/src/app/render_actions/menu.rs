use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn is_open_media_file_menu_action(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "File", "Open Media File")
    }

    pub(in crate::app) fn is_exit_menu_action(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "File", "Exit")
    }

    pub(in crate::app) fn direct_menu_actions(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<GuiShellAction>> {
        let actions = if Self::matches_menu_action(state, node, "Playback", "Seek") {
            vec![GuiShellAction::RequestSeekPrompt]
        } else if Self::matches_menu_action(state, node, "Playback", "Undo Seek") {
            vec![GuiShellAction::RequestPlaybackUndoSeek]
        } else if Self::matches_menu_action(state, node, "Advanced", "Set Offset") {
            vec![GuiShellAction::RequestOffsetPrompt]
        } else {
            return None;
        };
        Some(actions)
    }

    #[cfg(test)]
    pub(in crate::app) fn is_seek_menu_action(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "Playback", "Seek")
    }

    fn matches_menu_action(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
        section_title: &str,
        action_label: &str,
    ) -> bool {
        let Some((section_index, action_index)) = Self::menu_action_identity(node) else {
            return false;
        };
        let Some(section) = state.menus.sections.get(section_index) else {
            return false;
        };
        let Some(action) = section.actions.get(action_index) else {
            return false;
        };
        section.title == section_title && action.label == action_label
    }
}
