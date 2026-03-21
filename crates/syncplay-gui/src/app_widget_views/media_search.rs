use super::*;

impl SyncplayGuiShellAppState {
    pub(crate) fn media_search_widget_tree(&self) -> GuiWidgetNode {
        let selected_directory_index = self.selection.selected_media_search_directory;
        let can_manage_directories = self.pending_operation.is_none();
        let can_move_directory_up =
            can_manage_directories && selected_directory_index.is_some_and(|index| index > 0);
        let can_move_directory_down = can_manage_directories
            && selected_directory_index
                .is_some_and(|index| index + 1 < self.media_search.directories.len());
        let can_remove_directory = can_manage_directories && selected_directory_index.is_some();
        let directories = GuiWidgetNode::branch(
            "media-search:directories",
            "Directories",
            GuiWidgetKind::List,
            self.media_search
                .directories
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("media-search:directory:{index}"),
                        &row.path,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        );

        let utility_rail = GuiWidgetNode::layout(
            "media-search:utility",
            "Media Search Utility",
            GuiLayoutMode::Stack,
            vec![
                GuiWidgetNode::branch(
                    "media-search:commands",
                    "Commands",
                    GuiWidgetKind::Panel,
                    vec![GuiWidgetNode::layout(
                        "media-search:commands:buttons",
                        "Media Search Commands",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "media-search:command:browse",
                                "Browse Directories",
                                GuiWidgetKind::Button,
                                None,
                                self.media_search.can_browse_directories
                                    && self.pending_operation.is_none(),
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:command:search",
                                "Search Missing Media",
                                GuiWidgetKind::Button,
                                None,
                                self.commands.can_search_missing_media,
                                false,
                            ),
                        ],
                    )],
                ),
                GuiWidgetNode::branch(
                    "media-search:timing",
                    "Timing",
                    GuiWidgetKind::Panel,
                    vec![GuiWidgetNode::layout(
                        "media-search:timing:grid",
                        "Timing Grid",
                        GuiLayoutMode::KeyValueGrid {
                            min_pair_width: 220.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "media-search:timing:first-file",
                                "First File Timeout",
                                GuiWidgetKind::Status,
                                Some(optional_seconds_text(
                                    self.media_search.first_file_timeout_seconds,
                                )),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:timing:search",
                                "Search Timeout",
                                GuiWidgetKind::Status,
                                Some(optional_seconds_text(
                                    self.media_search.search_timeout_seconds,
                                )),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:timing:double-check",
                                "Double Check Interval",
                                GuiWidgetKind::Status,
                                Some(optional_seconds_text(
                                    self.media_search.double_check_interval_seconds,
                                )),
                                true,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:timing:warning-threshold",
                                "Warning Threshold",
                                GuiWidgetKind::Status,
                                Some(optional_seconds_text(
                                    self.media_search.warning_threshold_seconds,
                                )),
                                true,
                                false,
                            ),
                        ],
                    )],
                ),
                GuiWidgetNode::branch(
                    "media-search:directory-actions",
                    "Directory Actions",
                    GuiWidgetKind::Panel,
                    vec![GuiWidgetNode::layout(
                        "media-search:directory-actions:buttons",
                        "Directory Action Buttons",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![
                            GuiWidgetNode::leaf(
                                "media-search:directory:up",
                                "Move Selected Up",
                                GuiWidgetKind::Button,
                                None,
                                can_move_directory_up,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:directory:down",
                                "Move Selected Down",
                                GuiWidgetKind::Button,
                                None,
                                can_move_directory_down,
                                false,
                            ),
                            GuiWidgetNode::leaf(
                                "media-search:directory:remove",
                                "Remove Selected",
                                GuiWidgetKind::Button,
                                None,
                                can_remove_directory,
                                false,
                            ),
                        ],
                    )],
                ),
            ],
        );

        GuiWidgetNode::layout(
            "media-search-root",
            "Media Search",
            GuiLayoutMode::Stack,
            vec![GuiWidgetNode::layout(
                "media-search:content",
                "Media Search Content",
                GuiLayoutMode::ResponsiveColumns {
                    min_column_width: 360.0,
                    max_columns: 2,
                },
                vec![directories, utility_rail],
            )],
        )
    }
}
