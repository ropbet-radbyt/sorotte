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

#[test]
fn password_widget_debug_and_preview_redact_its_value() {
    let secret = "gui-password-widget-canary";
    let widget = GuiWidgetNode::leaf(
        "widget:password",
        "Password",
        GuiWidgetKind::PasswordInput,
        Some(secret.to_owned()),
        true,
        false,
    );

    let debug = format!("{widget:?}");
    assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
    assert!(!debug.contains(secret));

    let mut renderer = GuiWidgetTextPreviewRenderer::default();
    widget.render_with(&mut renderer);
    let preview = renderer.finish();
    assert!(preview.contains(sorotte_secret::REDACTED_SECRET));
    assert!(!preview.contains(secret));
}
