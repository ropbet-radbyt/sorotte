use super::*;

mod chat;
mod editors;
mod playlist;
mod summary;

impl SyncplayGuiShellAppState {
    pub(crate) fn main_window_widget_tree(&self) -> GuiWidgetNode {
        let (player_setup_panel, summary_column) = self.main_window_summary_projection();

        let playlist_column = self.main_window_playlist_column();
        let chat_panel = self.main_window_chat_panel();

        let top_region = GuiWidgetNode::layout(
            "main-window:top-region",
            "Room Dashboard",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 240.0,
                max_columns: 3,
            },
            vec![
                summary_column.clone(),
                playlist_column.clone(),
                chat_panel.clone(),
            ],
        );

        let mut overview_children = vec![top_region];
        let mut overview_editor_panels = self.main_window_editor_panels();
        if !overview_editor_panels.is_empty() {
            if overview_editor_panels.len() == 1
                && let Some(editor_panel) = overview_editor_panels.first_mut()
            {
                *editor_panel = editor_panel.clone().with_span(2);
            }
            overview_children.push(GuiWidgetNode::layout(
                "main-window:editors",
                "Room Editors",
                GuiLayoutMode::ResponsiveColumns {
                    min_column_width: 420.0,
                    max_columns: 2,
                },
                overview_editor_panels,
            ));
        }

        let overview_content = GuiWidgetNode::layout(
            "main-window:content",
            "Room Content",
            GuiLayoutMode::Stack,
            overview_children,
        );

        GuiWidgetNode::layout(
            "main-window-root",
            "Room",
            GuiLayoutMode::Stack,
            player_setup_panel
                .into_iter()
                .chain([overview_content])
                .collect(),
        )
    }
}
