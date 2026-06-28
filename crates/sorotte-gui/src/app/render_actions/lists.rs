use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn action_for_list_item_node(
        node: &GuiWidgetNode,
    ) -> Option<GuiShellAction> {
        match node.id.as_str() {
            "plugins:list:stream-support" => {
                return Some(GuiShellAction::SelectPlugin(
                    GuiPluginSelection::StreamSupport,
                ));
            }
            "plugins:list:media-matching" => {
                return Some(GuiShellAction::SelectPlugin(
                    GuiPluginSelection::MediaMatching,
                ));
            }
            "plugins:list:plex" => {
                return Some(GuiShellAction::SelectPlugin(GuiPluginSelection::Plex));
            }
            _ => {}
        }

        Self::parse_index_suffix(&node.id, "main-window:user:")
            .map(GuiShellAction::SelectMainWindowUser)
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "main-window:playlist:")
                    .map(GuiShellAction::SelectMainWindowPlaylist)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "main-window:playlist-plex-search:result:")
                    .map(GuiShellAction::SelectPlexPlaylistSearchResult)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "public-servers:row:")
                    .map(GuiShellAction::SelectPublicServer)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "media-search:directory:")
                    .map(GuiShellAction::SelectMediaSearchDirectory)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "shell:notification:")
                    .map(GuiShellAction::DismissTransientNotification)
            })
    }
}
