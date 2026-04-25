use super::super::widget_tree::{GuiWidgetNode, GuiWidgetRenderer};
use super::GuiWidgetEguiRenderer;

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
