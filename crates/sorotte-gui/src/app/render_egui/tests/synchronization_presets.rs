use super::*;
use crate::app::shell_state::SynchronizationPreset;

#[test]
fn preset_explanations_wrap_and_expose_complete_accessible_text() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::Overview,
    ));
    state.apply(GuiShellAction::ApplySynchronizationPreset(
        SynchronizationPreset::WatchTogether,
    ));
    let tree = state.configuration_widget_tree();
    let nodes = [
        "settings-preset:current",
        "settings-preset:description",
        "settings-preset:requirements",
    ]
    .map(|id| tree.find(id).unwrap());
    for width in [260.0, 700.0] {
        for zoom in [1.0, 1.5] {
            let context = egui::Context::default();
            context.enable_accesskit();
            context.set_zoom_factor(zoom);
            let mut renderer = GuiWidgetEguiRenderer::default();
            let mut right_edge = 0.0;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 1200.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(width / zoom);
                    ui.set_max_width(width / zoom);
                    right_edge = ui.max_rect().right();
                    for node in nodes {
                        renderer.render_status_pair(ui, node);
                    }
                },
            );
            output.textures_delta.clear();
            let accessibility = output.platform_output.accesskit_update.unwrap();
            for node in nodes {
                let expected = format!("{}: {}", node.label, node.value.as_deref().unwrap());
                assert!(
                    accessibility
                        .nodes
                        .iter()
                        .any(|(_, label)| label.value() == Some(expected.as_str())),
                    "complete preset status must be accessible: {expected}"
                );
                let text = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == expected => Some(text),
                        _ => None,
                    })
                    .expect("complete preset status must be rendered");
                assert!(
                    text.pos.x + text.galley.size().x <= right_edge + 1.0,
                    "preset status must fit at width {width}, zoom {zoom}"
                );
                if node.id != "settings-preset:current" && width < 300.0 {
                    assert!(
                        text.galley.rows.len() > 1,
                        "long explanations must wrap at width {width}, zoom {zoom}"
                    );
                }
            }
        }
    }
}
