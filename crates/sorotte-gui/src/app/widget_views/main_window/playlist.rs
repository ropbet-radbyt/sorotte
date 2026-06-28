use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn main_window_playlist_column(&self) -> GuiWidgetNode {
        let can_manage_playlist =
            self.main_window.playback.can_manage_playlist && self.pending_operation.is_none();
        let can_save_playlist =
            self.pending_operation.is_none() && !self.main_window.playlist.is_empty();
        let playlist_editor_active = self.playlist_text_edit_session.is_some()
            || self.playlist_url_edit_session.is_some()
            || self.plex_playlist_search.is_some();
        let playlist_has_entries = !self.main_window.playlist.is_empty();
        let controls_available = playlist_has_entries && self.pending_operation.is_none();
        let can_add_from_plex = can_manage_playlist
            && self.plugin_enablement.enabled_for(GuiPluginSelection::Plex)
            && self.plex.authenticated
            && self
                .plex
                .selected_server_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());

        let playlist_panel = GuiWidgetNode::branch(
            "main-window:playlist",
            "Entries",
            GuiWidgetKind::List,
            self.main_window
                .playlist
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let display_label = shared_playlist_display_label(&row.label);
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
                        display_label,
                        GuiWidgetKind::ListItem,
                        None,
                        true,
                        row.is_selected,
                    );
                    if let Some(source_button) = self.playlist_source_button_node(index) {
                        row_node.children.push(source_button);
                    }
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
        let add_plex_button = GuiWidgetNode::leaf(
            "main-window:playlist:add-plex",
            "Add from Plex...",
            GuiWidgetKind::Button,
            None,
            can_add_from_plex,
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
            vec![
                add_files_button,
                add_urls_button,
                add_plex_button,
                playlist_options_menu,
            ],
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

        let plex_playlist_search_panel = self.plex_playlist_search.as_ref().map(|search| {
            let helper_text = if search.searching {
                "Searching selected Plex server.".to_owned()
            } else if let Some(error) = search.error.as_deref() {
                error.to_owned()
            } else if search.results.is_empty() {
                "Search by title, or search empty to show recent Plex media.".to_owned()
            } else {
                format!("{} Plex results.", search.results.len())
            };
            let result_nodes = search
                .results
                .iter()
                .enumerate()
                .map(|(index, result)| {
                    let add_pending =
                        search.adding_rating_key.as_deref() == Some(result.rating_key.as_str());
                    let mut row = GuiWidgetNode::leaf(
                        format!("main-window:playlist-plex-search:result:{index}"),
                        plex_playlist_search_result_label(result),
                        GuiWidgetKind::ListItem,
                        result.file_name.clone(),
                        !search.searching,
                        search.selected_index == Some(index),
                    );
                    row.children.push(GuiWidgetNode::leaf(
                        format!("main-window:playlist-plex-search:result:{index}:add"),
                        if add_pending { "Adding..." } else { "Add" },
                        GuiWidgetKind::Button,
                        None,
                        can_manage_playlist
                            && !search.searching
                            && search.adding_rating_key.is_none(),
                        false,
                    ));
                    row
                })
                .collect();
            GuiWidgetNode::branch(
                "main-window:playlist-plex-search",
                "Add from Plex",
                GuiWidgetKind::Panel,
                vec![
                    GuiWidgetNode::leaf(
                        "main-window:playlist-plex-search:close",
                        "Close",
                        GuiWidgetKind::Button,
                        None,
                        true,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-plex-search:query",
                        "Search Plex",
                        GuiWidgetKind::TextInput,
                        Some(search.query.clone()),
                        can_manage_playlist && !search.searching,
                        false,
                    ),
                    GuiWidgetNode::layout(
                        "main-window:playlist-plex-search:actions",
                        "Plex Search Actions",
                        GuiLayoutMode::ButtonWrap {
                            min_button_width: 120.0,
                        },
                        vec![GuiWidgetNode::leaf(
                            "main-window:playlist-plex-search:submit",
                            if search.query.trim().is_empty() {
                                "Show Recent"
                            } else {
                                "Search"
                            },
                            GuiWidgetKind::Button,
                            None,
                            can_manage_playlist && !search.searching,
                            false,
                        )],
                    ),
                    GuiWidgetNode::leaf(
                        "main-window:playlist-plex-search:helper",
                        "Status",
                        GuiWidgetKind::Status,
                        Some(helper_text),
                        true,
                        false,
                    ),
                    GuiWidgetNode::branch(
                        "main-window:playlist-plex-search:results",
                        "Plex Results",
                        GuiWidgetKind::List,
                        result_nodes,
                    )
                    .with_min_content_height(140.0),
                ],
            )
        });

        let mut playlist_default_source_button = GuiWidgetNode::branch(
            "main-window:playlist-default-source",
            format!(
                "Default Source: {}",
                self.main_window.playlist_default_source.current_label
            ),
            GuiWidgetKind::Button,
            self.main_window
                .playlist_default_source
                .options
                .iter()
                .map(|option| {
                    let mut option_node = GuiWidgetNode::leaf(
                        format!(
                            "main-window:playlist-default-source:{}",
                            option.source_id.as_action_id()
                        ),
                        option.label.clone(),
                        GuiWidgetKind::Button,
                        Some(option.status.label().to_owned()),
                        option.enabled,
                        option.selected,
                    );
                    if let Some(detail) = option.detail.as_ref() {
                        option_node = option_node.with_tooltip(detail.clone());
                    }
                    option_node
                })
                .collect(),
        );
        playlist_default_source_button.value = Some(
            self.main_window
                .playlist_default_source
                .options
                .iter()
                .find(|option| option.selected)
                .map(|option| option.status.label().to_owned())
                .unwrap_or_else(|| "available".to_owned()),
        );
        playlist_default_source_button = playlist_default_source_button.with_tooltip(
            playlist_default_source_tooltip(&self.main_window.playlist_default_source),
        );
        let playlist_default_source_footer = GuiWidgetNode::layout(
            "main-window:playlist-default-source-row",
            "Default Source",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 180.0,
            },
            vec![playlist_default_source_button],
        );

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
                plex_playlist_search_panel.clone(),
            ])
            .chain([Some(playlist_panel.clone())])
            .chain([Some(playlist_default_source_footer.clone())])
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

fn playlist_default_source_tooltip(source_state: &GuiPlaylistDefaultSourceState) -> String {
    let mut lines = vec![
        format!(
            "Default source for new items: {}",
            source_state.current_label
        ),
        "Existing playlist items keep their own selected source.".to_owned(),
    ];
    if !source_state.options.is_empty() {
        lines.push("Available defaults:".to_owned());
        lines.extend(source_state.options.iter().map(|option| {
            let mut line = format!("- {}: {}", option.label, option.status.label());
            if let Some(detail) = option.detail.as_deref() {
                line.push_str(" - ");
                line.push_str(detail);
            }
            line
        }));
    }
    lines.join("\n")
}

fn shared_playlist_display_label(entry: &str) -> String {
    let trimmed = entry.trim();
    let Ok(uri) = sorotte_plex::parse_plex_playlist_uri(trimmed) else {
        return sorotte_plex::redact_plex_token(entry);
    };
    uri.file_name
        .as_deref()
        .and_then(non_empty_display_text)
        .or_else(|| uri.title.as_deref().and_then(non_empty_display_text))
        .unwrap_or_else(|| sorotte_plex::redact_plex_token(trimmed))
}

fn non_empty_display_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn plex_playlist_search_result_label(result: &GuiPlexPlaylistSearchResult) -> String {
    let mut parts = Vec::new();
    push_unique_label_part(&mut parts, result.grandparent_title.as_deref());
    push_unique_label_part(&mut parts, result.parent_title.as_deref());
    push_unique_label_part(&mut parts, Some(&result.title));
    let mut label = if parts.is_empty() {
        "Untitled".to_owned()
    } else {
        parts.join(" - ")
    };
    if let Some(duration) = result.duration_millis.and_then(duration_millis_text) {
        label.push_str(" (");
        label.push_str(&duration);
        label.push(')');
    }
    if let Some(file_name) = result.file_name.as_deref().and_then(non_empty_display_text) {
        label.push_str(" | ");
        label.push_str(&file_name);
    }
    label
}

fn push_unique_label_part(parts: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.and_then(non_empty_display_text) else {
        return;
    };
    if !parts.iter().any(|part| part == &value) {
        parts.push(value);
    }
}

fn duration_millis_text(value: u64) -> Option<String> {
    if value == 0 {
        return None;
    }
    let total_seconds = value / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    Some(format!("{minutes}:{seconds:02}"))
}
