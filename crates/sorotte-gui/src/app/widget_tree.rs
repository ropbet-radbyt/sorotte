#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum GuiLayoutMode {
    Stack,
    ResponsiveColumns {
        min_column_width: f32,
        max_columns: usize,
    },
    TabStrip {
        min_tab_width: f32,
    },
    FormGrid {
        label_width: f32,
        min_field_width: f32,
    },
    KeyValueGrid {
        min_pair_width: f32,
    },
    ButtonWrap {
        min_button_width: f32,
    },
    CompactButtonWrap {
        button_width: f32,
        button_height: f32,
        gap: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiWidgetKind {
    Layout,
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
            Self::Layout => "layout",
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

#[derive(Clone, PartialEq)]
pub(super) struct GuiWidgetNode {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) kind: GuiWidgetKind,
    pub(super) value: Option<String>,
    pub(super) enabled: bool,
    pub(super) selected: bool,
    pub(super) tooltip: Option<String>,
    pub(super) layout_mode: Option<GuiLayoutMode>,
    pub(super) column_span: usize,
    pub(super) min_content_height: Option<f32>,
    pub(super) children: Vec<GuiWidgetNode>,
}

impl std::fmt::Debug for GuiWidgetNode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = if self.value_is_sensitive_for_diagnostics() && self.value.is_some() {
            Some(sorotte_secret::REDACTED_SECRET)
        } else {
            self.value.as_deref()
        };
        formatter
            .debug_struct("GuiWidgetNode")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("value", &value)
            .field("enabled", &self.enabled)
            .field("selected", &self.selected)
            .field("tooltip", &self.tooltip)
            .field("layout_mode", &self.layout_mode)
            .field("children", &self.children)
            .finish()
    }
}

impl GuiWidgetNode {
    fn value_is_sensitive_for_diagnostics(&self) -> bool {
        self.kind == GuiWidgetKind::PasswordInput
            || (self.kind == GuiWidgetKind::ListItem && self.id.starts_with("main-window:chat:"))
    }

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
            tooltip: None,
            layout_mode: None,
            column_span: 1,
            min_content_height: None,
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
            tooltip: None,
            layout_mode: None,
            column_span: 1,
            min_content_height: None,
            children,
        }
    }

    pub(super) fn layout(
        id: impl Into<String>,
        label: impl Into<String>,
        mode: GuiLayoutMode,
        children: Vec<GuiWidgetNode>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: GuiWidgetKind::Layout,
            value: None,
            enabled: true,
            selected: false,
            tooltip: None,
            layout_mode: Some(mode),
            column_span: 1,
            min_content_height: None,
            children,
        }
    }

    pub(super) fn with_span(mut self, column_span: usize) -> Self {
        self.column_span = column_span.max(1);
        self
    }

    pub(super) fn with_min_content_height(mut self, min_content_height: f32) -> Self {
        self.min_content_height = Some(min_content_height.max(0.0));
        self
    }

    pub(super) fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub(super) fn find(&self, id: &str) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    #[cfg(any(
        feature = "gui-semantic-smoke",
        all(test, feature = "live-python-interop")
    ))]
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
        let value = if node.value_is_sensitive_for_diagnostics() && node.value.is_some() {
            sorotte_secret::REDACTED_SECRET
        } else {
            node.value.as_deref().unwrap_or("(none)")
        };
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
