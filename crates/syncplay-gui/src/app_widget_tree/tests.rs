use super::{GuiWidgetKind, GuiWidgetNode, GuiWidgetTextPreviewRenderer};

#[test]
fn gui_widget_text_preview_renderer_formats_widget_nodes() {
    let mut renderer = GuiWidgetTextPreviewRenderer::default();
    let widget = GuiWidgetNode::leaf(
        "widget:test",
        "Test Widget",
        GuiWidgetKind::Button,
        Some("click".to_owned()),
        false,
        true,
    );

    widget.render_with(&mut renderer);

    assert_eq!(
        renderer.finish(),
        "- Test Widget [button] id=widget:test, enabled=no, selected=yes, value=click"
    );
}
