use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn main_window_playlist_column(&self) -> GuiWidgetNode {
        let can_manage_playlist =
            self.main_window.playback.can_manage_playlist && self.pending_operation.is_none();
        let selected_playlist_index = self.selection.selected_main_window_playlist;
        let can_remove_playlist = can_manage_playlist && selected_playlist_index.is_some();
        let selected_playlist_entry = self.selected_shared_playlist_entry().map(str::to_owned);
        let selected_playlist_is_url = selected_playlist_entry
            .as_deref()
            .is_some_and(browser_is_url);
        let trusted_domains = self
            .configuration
            .to_stored_settings()
            .trusted_domains
            .unwrap_or_default();
        let selected_playlist_domain = selected_playlist_entry
            .as_deref()
            .and_then(browser_domain_from_url);
        let can_open_selected_playlist =
            self.pending_operation.is_none() && selected_playlist_entry.is_some();
        let can_open_selected_playlist_folder = self.pending_operation.is_none()
            && selected_playlist_entry.is_some()
            && !selected_playlist_is_url;
        let can_trust_selected_playlist_domain = self.pending_operation.is_none()
            && selected_playlist_domain.is_some()
            && selected_playlist_entry
                .as_deref()
                .is_some_and(|entry| !browser_uri_is_trusted(entry, true, &trusted_domains));
        let can_save_playlist =
            self.pending_operation.is_none() && !self.main_window.playlist.is_empty();
        let can_shuffle_remaining = can_manage_playlist
            && selected_playlist_index
                .is_some_and(|index| index + 1 < self.main_window.playlist.len());
        let can_shuffle_entire = can_manage_playlist && !self.main_window.playlist.is_empty();
        let can_undo_playlist = can_manage_playlist
            && self
                .playlist_undo_snapshot
                .as_ref()
                .is_some_and(|previous| *previous != self.current_shared_playlist_entries());

        let playlist_panel = GuiWidgetNode::branch(
            "main-window:playlist",
            "Entries",
            GuiWidgetKind::List,
            self.main_window
                .playlist
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    GuiWidgetNode::leaf(
                        format!("main-window:playlist:{index}"),
                        &row.label,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    )
                })
                .collect(),
        )
        .with_min_content_height(220.0);

        let mut playlist_add_menu = GuiWidgetNode::branch(
            "main-window:playlist:add-menu",
            "Add",
            GuiWidgetKind::Button,
            vec![
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-files",
                    "Choose Files...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:add-url",
                    "Paste URLs...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
            ],
        );
        playlist_add_menu.enabled = can_manage_playlist;

        let mut playlist_more_menu = GuiWidgetNode::branch(
            "main-window:playlist:more-menu",
            "More",
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
                GuiWidgetNode::leaf(
                    "main-window:playlist:load-shuffle",
                    "Load + Shuffle...",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:undo",
                    "Undo",
                    GuiWidgetKind::Button,
                    None,
                    can_undo_playlist,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:shuffle-remaining",
                    "Shuffle Remaining",
                    GuiWidgetKind::Button,
                    None,
                    can_shuffle_remaining,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:shuffle-entire",
                    "Shuffle Entire",
                    GuiWidgetKind::Button,
                    None,
                    can_shuffle_entire,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "main-window:playlist:edit",
                    "Edit Playlist",
                    GuiWidgetKind::Button,
                    None,
                    can_manage_playlist,
                    false,
                ),
            ],
        );
        playlist_more_menu.enabled = !playlist_more_menu.children.is_empty();

        let playlist_header = GuiWidgetNode::layout(
            "main-window:playlist-header:actions",
            "Playlist Header Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 118.0,
            },
            vec![playlist_add_menu, playlist_more_menu],
        );

        let playlist_selection_bar = selected_playlist_entry.as_ref().map(|_| {
            let mut selection_actions = vec![GuiWidgetNode::leaf(
                "main-window:playlist:open-selected",
                "Open",
                GuiWidgetKind::Button,
                None,
                can_open_selected_playlist,
                false,
            )];
            if can_open_selected_playlist_folder {
                selection_actions.push(GuiWidgetNode::leaf(
                    "main-window:playlist:open-selected-folder",
                    "Open Folder",
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                ));
            }
            if can_trust_selected_playlist_domain {
                selection_actions.push(GuiWidgetNode::leaf(
                    "main-window:playlist:trust-selected",
                    selected_playlist_domain
                        .as_deref()
                        .map(|domain| format!("Trust {domain}"))
                        .unwrap_or_else(|| "Trust Selected Domain".to_owned()),
                    GuiWidgetKind::Button,
                    None,
                    true,
                    false,
                ));
            }
            selection_actions.push(GuiWidgetNode::leaf(
                "main-window:playlist:remove",
                "Remove",
                GuiWidgetKind::Button,
                None,
                can_remove_playlist,
                false,
            ));

            GuiWidgetNode::layout(
                "main-window:playlist-selection:actions",
                "Selected Playlist Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 140.0,
                },
                selection_actions,
            )
        });

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
            GuiWidgetNode::branch(
                "main-window:playlist-url-edit",
                "Playlist URLs",
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
                            session.is_dirty,
                            false,
                        )],
                    ),
                ],
            )
        });

        let mut playlist_column_children = vec![GuiWidgetNode::branch(
            "main-window:playlist-surface",
            "Shared Playlist",
            GuiWidgetKind::Panel,
            [playlist_header.clone()]
                .into_iter()
                .chain([playlist_panel.clone()])
                .chain(playlist_selection_bar.clone())
                .collect(),
        )];
        playlist_column_children.extend(
            [
                playlist_text_edit_panel.clone(),
                playlist_url_edit_panel.clone(),
            ]
            .into_iter()
            .flatten(),
        );
        GuiWidgetNode::layout(
            "main-window:playlist-column",
            "Playlist Column",
            GuiLayoutMode::Stack,
            playlist_column_children,
        )
    }
}
