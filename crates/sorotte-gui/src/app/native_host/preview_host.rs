use super::*;

#[cfg(test)]
impl GuiAppHost for GuiTextPreviewHost {
    type Output = String;

    fn render(&mut self, state: SorotteGuiShellAppState) -> Self::Output {
        let mut renderer = GuiWidgetTextPreviewRenderer::default();
        state.render_shell_widgets(&mut renderer);
        format!(
            "{}\n\n[Widget Tree]\n{}",
            state.render_lines().join("\n"),
            renderer.finish()
        )
    }
}
