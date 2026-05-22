use eframe::egui;

use super::super::widget_tree::GuiWidgetNode;
use super::GuiWidgetEguiRenderer;

impl GuiWidgetEguiRenderer {
    pub(super) fn display_text(node: &GuiWidgetNode) -> String {
        match node.value.as_deref() {
            Some(value) if !value.is_empty() => format!("{}: {}", node.label, value),
            _ => node.label.clone(),
        }
    }

    pub(super) fn display_status_value(node: &GuiWidgetNode) -> String {
        let value = node.value.as_deref().unwrap_or("(none)");
        match (node.id.as_str(), value) {
            ("main-window:connection-status", "connected") => "Connected".to_owned(),
            ("main-window:connection-status", "connecting") => "Connecting".to_owned(),
            ("main-window:connection-status", "disconnecting") => "Disconnecting".to_owned(),
            ("main-window:connection-status", "disconnected") => "Disconnected".to_owned(),
            ("main-window:connection-status", "not-configured") => "Not configured".to_owned(),
            ("main-window:playback-paused", "yes" | "true") => "Paused".to_owned(),
            ("main-window:playback-paused", "no" | "false") => "Playing".to_owned(),
            ("main-window:autoplay", "yes" | "true") => "On".to_owned(),
            ("main-window:autoplay", "no" | "false") => "Off".to_owned(),
            ("main-window:user-offset", value) => {
                Self::format_offset_seconds(value).unwrap_or_else(|| value.to_owned())
            }
            _ => value.to_owned(),
        }
    }

    pub(super) fn display_status_rich_text(ui: &egui::Ui, node: &GuiWidgetNode) -> egui::RichText {
        let text = Self::display_status_value(node);
        let palette = Self::palette_for_ui(ui);
        let color = match (node.id.as_str(), node.value.as_deref().unwrap_or("(none)")) {
            ("main-window:connection-status", "connected") => Some(palette.success_text),
            ("main-window:connection-status", "connecting" | "disconnecting") => {
                Some(palette.warning_text)
            }
            ("main-window:connection-status", "disconnected" | "not-configured") => {
                Some(palette.neutral_text)
            }
            ("main-window:playback-paused", "yes" | "true") => Some(palette.danger),
            ("main-window:playback-paused", "no" | "false") => Some(palette.success_text),
            ("main-window:autoplay", "yes" | "true") => Some(palette.success_text),
            ("main-window:autoplay", "no" | "false") => Some(palette.neutral_text),
            _ => None,
        };
        let rich_text = egui::RichText::new(text);
        if let Some(color) = color {
            rich_text.color(color).strong()
        } else {
            rich_text
        }
    }

    fn format_offset_seconds(value: &str) -> Option<String> {
        let seconds = value.parse::<f64>().ok()?;
        let sign = if seconds.is_sign_negative() { "-" } else { "" };
        let total_millis = (seconds.abs() * 1000.0).round() as u64;
        let total_seconds = total_millis / 1000;
        let millis = total_millis % 1000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        if millis > 0 {
            if hours > 0 {
                Some(format!(
                    "{sign}{hours:02}:{minutes:02}:{seconds:02}.{millis:03}"
                ))
            } else {
                Some(format!("{sign}{minutes:02}:{seconds:02}.{millis:03}"))
            }
        } else if hours > 0 {
            Some(format!("{sign}{hours:02}:{minutes:02}:{seconds:02}"))
        } else {
            Some(format!("{sign}{minutes:02}:{seconds:02}"))
        }
    }

    pub(super) fn should_render_combined_status_label(node: &GuiWidgetNode) -> bool {
        node.id.starts_with("media-search:timing:")
            || node.id.starts_with("shell:command:")
            || node.id.starts_with("shell:validation:")
    }

    pub(super) fn is_surface_node(node: &GuiWidgetNode) -> bool {
        matches!(
            node.id.as_str(),
            "configuration-root" | "main-window-root" | "plugins-root"
        )
    }
}
