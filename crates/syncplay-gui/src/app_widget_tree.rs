#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiWidgetKind {
    Panel,
    TextInput,
    TextArea,
    PasswordInput,
    Checkbox,
    Select,
    NumericInput,
    ReadOnly,
    Button,
    List,
    ListItem,
    Status,
}

impl GuiWidgetKind {
    #[cfg(test)]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::TextInput => "text-input",
            Self::TextArea => "text-area",
            Self::PasswordInput => "password-input",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::NumericInput => "numeric-input",
            Self::ReadOnly => "read-only",
            Self::Button => "button",
            Self::List => "list",
            Self::ListItem => "list-item",
            Self::Status => "status",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiWidgetNode {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: GuiWidgetKind,
    pub(super) value: Option<String>,
    pub(super) enabled: bool,
    pub(super) selected: bool,
    pub(super) children: Vec<GuiWidgetNode>,
}

impl GuiWidgetNode {
    pub(super) fn leaf(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: GuiWidgetKind,
        value: Option<String>,
        enabled: bool,
        selected: bool,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            value,
            enabled,
            selected,
            children: Vec::new(),
        }
    }

    pub(super) fn branch(
        id: impl Into<String>,
        label: impl Into<String>,
        kind: GuiWidgetKind,
        children: Vec<GuiWidgetNode>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            value: None,
            enabled: true,
            selected: false,
            children,
        }
    }

    pub(super) fn find(&self, id: &str) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    pub(super) fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(GuiWidgetNode::node_count)
            .sum::<usize>()
    }

    pub(super) fn render_with(&self, renderer: &mut impl GuiWidgetRenderer) {
        self.render_with_depth(renderer, 0);
    }

    fn render_with_depth(&self, renderer: &mut impl GuiWidgetRenderer, depth: usize) {
        renderer.begin_node(self, depth);
        for child in &self.children {
            child.render_with_depth(renderer, depth + 1);
        }
        renderer.end_node(self, depth);
    }
}

pub(super) trait GuiWidgetRenderer {
    fn begin_node(&mut self, node: &GuiWidgetNode, depth: usize);

    fn end_node(&mut self, node: &GuiWidgetNode, depth: usize);
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct GuiWidgetTextPreviewRenderer {
    lines: Vec<String>,
}

#[cfg(test)]
impl GuiWidgetTextPreviewRenderer {
    pub(super) fn finish(self) -> String {
        self.lines.join("\n")
    }
}

#[cfg(test)]
impl GuiWidgetRenderer for GuiWidgetTextPreviewRenderer {
    fn begin_node(&mut self, node: &GuiWidgetNode, depth: usize) {
        let indent = "  ".repeat(depth);
        let value = node.value.as_deref().unwrap_or("(none)");
        self.lines.push(format!(
            "{indent}- {} [{}] id={}, enabled={}, selected={}, value={value}",
            node.label,
            node.kind.label(),
            node.id,
            widget_bool_label(node.enabled),
            widget_bool_label(node.selected),
        ));
    }

    fn end_node(&mut self, _node: &GuiWidgetNode, _depth: usize) {}
}

#[cfg(test)]
fn widget_bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
