use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn main_window_playlist_column(&self) -> GuiWidgetNode {
        let can_manage_playlist =
            self.main_window.playback.can_manage_playlist && self.pending_operation.is_none();
        let can_save_playlist =
            self.pending_operation.is_none() && !self.main_window.playlist.is_empty();
        let playlist_editor_active =
            self.playlist_text_edit_session.is_some() || self.playlist_url_edit_session.is_some();
        let playlist_has_entries = !self.main_window.playlist.is_empty();
        let controls_available = playlist_has_entries && self.pending_operation.is_none();

        let playlist_panel = GuiWidgetNode::branch(
            "main-window:playlist",
            "Entries",
            GuiWidgetKind::List,
            self.main_window
                .playlist
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let remove_button = GuiWidgetNode::leaf(
                        format!("main-window:playlist:{index}:remove"),
                        "Remove",
                        GuiWidgetKind::Button,
                        None,
                        can_manage_playlist,
                        false,
                    );
                    let mut row_node = GuiWidgetNode::leaf(
                        format!("main-window:playlist:{index}"),
                        &row.label,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    );
                    row_node.children.push(remove_button);
                    row_node
                })
                .collect(),
        )
        .with_min_content_height(if playlist_editor_active { 80.0 } else { 220.0 });

        let add_files_button = GuiWidgetNode::leaf(
            "main-window:playlist:add-files",
            "Choose Files...",
            GuiWidgetKind::Button,
            None,
            can_manage_playlist,
            false,
        );
        let add_urls_button = GuiWidgetNode::leaf(
            "main-window:playlist:add-url",
            "Paste URLs...",
            GuiWidgetKind::Button,
            None,
            can_manage_playlist,
            false,
        );
        let mut playlist_options_menu = GuiWidgetNode::branch(
            "main-window:playlist:more-menu",
            "Playlist Options",
            GuiWidgetKind::Button,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playlist:load",
                    "Load Playlist...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:save",
                    "Save Playlist...",
                    GuiWidgetKind::Button,
                    None,
                    can_save_playlist,
                    false,
                ),
            ],
        );
        playlist_options_menu.enabled = can_manage_playlist || can_save_playlist;

        let playlist_header = GuiWidgetNode::layout(
            "main-window:playlist-header:actions",
            "Playlist Header Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 40.0,
            },
            vec![add_files_button, add_urls_button, playlist_options_menu],
        );

        let playlist_text_edit_panel = self.playlist_text_edit_session.as_ref().map(|session| {
            GuiWidgetNode::branch(
                "main-window:playlist-edit",
                "Playlist Editor",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:close",
                        "Close",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-edit:text",
                        "Playlist Entries",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:playlist-edit:actions",
                        "Playlist Editor Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:playlist-edit:commit",
                            "Apply Playlist",
                            GuiWidgetKind::Button,
                            None,
                            session.is_dirty,
                            false,
                        )],
                    ),
                ],
            )
        });

        let playlist_url_edit_panel = self.playlist_url_edit_session.as_ref().map(|session| {
            let detected_entries = playlist_entries_from_multiline_text(&session.buffer);
            let detected_count = detected_entries.len();
            let helper_text = if detected_count == 0 {
                "Paste one URL per line.".to_owned()
            } else if detected_count == 1 {
                "1 URL detected.".to_owned()
            } else {
                format!("{detected_count} URLs detected.")
            };
            GuiWidgetNode::branch(
                "main-window:playlist-url-edit",
                "Add URLs",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:close",
                        "Close",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:text",
                        "URLs",
                        GuiWidgetKind::TextArea,
                        Some(session.buffer.clone()),
                        can_manage_playlist,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-url-edit:helper",
                        "Helper",
                        GuiWidgetKind::Status,
                        Some(helper_text),
                        true,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:playlist-url-edit:actions",
                        "Playlist URL Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 140.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:playlist-url-edit:commit",
                            "Add URLs To Playlist",
                            GuiWidgetKind::Button,
                            None,
                            session.is_dirty && detected_count > 0,
                            false,
                        )],
                    ),
                ],
            )
        });

        let mut control_buttons = Vec::new();
        if self.main_window.show_playback_buttons {
            control_buttons.extend([
                GuiWidgetNode::leaf(
                    "main-window:control:play",
                    "Play",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:pause",
                    "Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:toggle-pause",
                    "Toggle Pause",
                    GuiWidgetKind::Button,
                    None,
                    self.commands.can_toggle_pause && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:seek",
                    "Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_seek && controls_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:control:undo-seek",
                    "Undo Seek",
                    GuiWidgetKind::Button,
                    None,
                    self.main_window.playback.can_undo_seek && controls_available,
                    false,
                ),
            ]);
        }
        let playlist_playback_footer = GuiWidgetNode::layout(
            "main-window:playlist-playback",
            "Playback",
            GuiLayoutMode::Stack,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playback-paused",
                    "Playback",
                    GuiWidgetKind::Status,
                    Some(bool_label(self.main_window.playback_paused).to_owned()),
                    true,
                    false,
                ),
                GuiWidgetNode::layout(
                    "main-window:controls:playback-actions",
                    "Playback Controls",
                    GuiLayoutMode::CompactButtonWrap {
                        button_width: 40.0,
                        button_height: 36.0,
                        gap: 8.0,
                    },
                    control_buttons,
                ),
            ],
        );

        let playlist_surface_children = [Some(playlist_header.clone())]
            .into_iter()
            .chain([
                playlist_text_edit_panel.clone(),
                playlist_url_edit_panel.clone(),
            ])
            .chain([Some(playlist_panel.clone())])
            .chain([Some(playlist_playback_footer.clone())])
            .flatten()
            .collect();
        let playlist_column_children = vec![
            GuiWidgetNode::branch(
                "main-window:playlist-surface",
                "Playlist",
                GuiWidgetKind::Panel,
                playlist_surface_children,
            )
            .with_min_content_height(if playlist_editor_active {
                520.0
            } else {
                420.0
            }),
        ];
        GuiWidgetNode::layout(
            "main-window:playlist-column",
            "Playlist Column",
            GuiLayoutMode::Stack,
            playlist_column_children,
        )
    }
}
