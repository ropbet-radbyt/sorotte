use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app::render_actions) fn configuration_control_identity(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<(&'static str, &'static str, GuiDialogControlKind)> {
        let identity = node.id.strip_prefix("config:")?;
        let (section, label) = identity.split_once(':')?;
        state.configuration.control_identity(section, label)
    }

    pub(in crate::app::render_actions) fn menu_action_identity(
        node: &GuiWidgetNode,
    ) -> Option<(usize, usize)> {
        let identity = node.id.strip_prefix("menus:action:")?;
        let (section_index, action_index) = identity.split_once(':')?;
        Some((section_index.parse().ok()?, action_index.parse().ok()?))
    }

    pub(in crate::app::render_actions) fn main_window_room_draft(
        state: &SorotteGuiShellAppState,
    ) -> String {
        state
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default()
            .to_owned()
    }

    pub(in crate::app::render_actions) fn parse_index_suffix(
        id: &str,
        prefix: &str,
    ) -> Option<usize> {
        id.strip_prefix(prefix)?.parse().ok()
    }

    pub(in crate::app::render_actions) fn main_window_browser_room_action_index(
        id: &str,
        action: &str,
    ) -> Option<usize> {
        let identity = id.strip_prefix("main-window:room-group:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    pub(in crate::app::render_actions) fn main_window_browser_user_action_index(
        id: &str,
        action: &str,
    ) -> Option<usize> {
        let identity = id.strip_prefix("main-window:user:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    pub(in crate::app::render_actions) fn main_window_playlist_row_action_index(
        id: &str,
        action: &str,
    ) -> Option<usize> {
        let identity = id.strip_prefix("main-window:playlist:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    pub(in crate::app::render_actions) fn main_window_playlist_source_action(
        id: &str,
    ) -> Option<(usize, GuiMediaSourceProviderId)> {
        let identity = id.strip_prefix("main-window:playlist:")?;
        let (index, suffix) = identity.split_once(':')?;
        let provider_id = suffix.strip_prefix("source:")?;
        Some((
            index.parse().ok()?,
            GuiMediaSourceProviderId::new(provider_id.to_owned()),
        ))
    }

    pub(in crate::app::render_actions) fn plex_playlist_search_result_action_index(
        id: &str,
        action: &str,
    ) -> Option<usize> {
        let identity = id.strip_prefix("main-window:playlist-plex-search:result:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }
}
