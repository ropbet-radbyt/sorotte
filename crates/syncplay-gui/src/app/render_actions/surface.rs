use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn action_for_surface_node(node: &GuiWidgetNode) -> Option<GuiShellAction> {
        let view = match node.id.as_str() {
            "configuration-root" => GuiShellView::Setup,
            "main-window-root" => GuiShellView::Room,
            _ => return None,
        };
        Some(GuiShellAction::SwitchView(view))
    }
}
