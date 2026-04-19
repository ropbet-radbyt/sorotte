use syncplay_client_app::app_boundary::{
    commands::{
        controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
    },
    language::SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
};

use super::mpv_launch;
use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::{
    GuiConfigurationTab, GuiDialogControlKind, GuiDraftRuntimeSnapshot, GuiMainWindowTab,
    GuiShellAction, GuiShellModal, GuiShellView, GuiTransientNotificationLevel,
    SyncplayGuiShellAppState, browser_domain_from_url, load_playlist_entries_from_path,
    playlist_entries_from_multiline_text, save_playlist_entries_to_path,
};
use super::support::{nonempty_room_name_text, normalized_editable_text};
use super::widget_tree::GuiWidgetNode;

impl GuiWidgetEguiRenderer {
    pub(super) fn action_for_surface_node(node: &GuiWidgetNode) -> Option<GuiShellAction> {
        let view = match node.id.as_str() {
            "configuration-root" => GuiShellView::Configuration,
            "main-window-root" => GuiShellView::MainWindow,
            "menus-root" => GuiShellView::MenusAndDialogs,
            "public-servers-root" => GuiShellView::PublicServers,
            "media-search-root" => GuiShellView::MediaSearch,
            _ => return None,
        };
        Some(GuiShellAction::SwitchView(view))
    }

    pub(super) fn actions_for_button_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Vec<GuiShellAction> {
        if let Some(room_index) = Self::main_window_browser_room_action_index(&node.id, "join") {
            return state
                .main_window
                .rooms
                .get(room_index)
                .map(|room| vec![GuiShellAction::JoinMainWindowRoom(room.room_name.clone())])
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "open") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.clone())
                .map(|target| vec![GuiShellAction::RequestMainWindowUserMediaOpen(target)])
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "folder") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.clone())
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
                        target,
                    )]
                })
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "ready") {
            return state
                .main_window
                .users
                .get(user_index)
                .filter(|user| state.can_request_main_window_user_ready_change(user))
                .map(|user| {
                    vec![GuiShellAction::RequestMainWindowUserReady {
                        username: user.username.clone(),
                        ready: !user.is_ready,
                    }]
                })
                .unwrap_or_default();
        }
        if let Some(user_index) = Self::main_window_browser_user_action_index(&node.id, "trust") {
            return state
                .main_window
                .users
                .get(user_index)
                .and_then(|user| user.file_name.as_deref())
                .and_then(browser_domain_from_url)
                .map(|domain| vec![GuiShellAction::AddTrustedDomain(domain)])
                .unwrap_or_default();
        }
        if node.id == "main-window:playlist:add-files" {
            return Self::pick_media_files(state)
                .map(Self::shared_playlist_entries_for_media_paths)
                .map(GuiShellAction::AppendSharedPlaylistEntries)
                .into_iter()
                .collect();
        }
        if matches!(
            node.id.as_str(),
            "main-window:playlist:load" | "main-window:playlist:load-shuffle"
        ) {
            let Some(path) = Self::pick_playlist_load_file(state) else {
                return Vec::new();
            };
            return match load_playlist_entries_from_path(&path) {
                Ok(entries) => vec![GuiShellAction::LoadSharedPlaylistFromFile {
                    path,
                    entries,
                    shuffled: node.id == "main-window:playlist:load-shuffle",
                }],
                Err(error) => vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(error),
                ],
            };
        }
        if node.id == "main-window:playlist:save" {
            let Some(path) = Self::pick_playlist_save_file(state) else {
                return Vec::new();
            };
            let entries = state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.clone())
                .collect::<Vec<_>>();
            return match save_playlist_entries_to_path(&path, &entries) {
                Ok(()) => vec![GuiShellAction::SaveSharedPlaylistToFile(path)],
                Err(error) => vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(error),
                ],
            };
        }

        match node.id.as_str() {
            "main-window:tab:overview" => {
                vec![GuiShellAction::SelectMainWindowTab(
                    GuiMainWindowTab::Overview,
                )]
            }
            "main-window:tab:session" => {
                vec![GuiShellAction::SelectMainWindowTab(
                    GuiMainWindowTab::Session,
                )]
            }
            "main-window:tab:playback" => {
                vec![GuiShellAction::SelectMainWindowTab(
                    GuiMainWindowTab::Playback,
                )]
            }
            "main-window:tab:playlist" => {
                vec![GuiShellAction::SelectMainWindowTab(
                    GuiMainWindowTab::Playlist,
                )]
            }
            "main-window:tab:chat" => {
                vec![GuiShellAction::SelectMainWindowTab(GuiMainWindowTab::Chat)]
            }
            "configuration:tab:overview" => {
                vec![GuiShellAction::SelectConfigurationTab(
                    GuiConfigurationTab::Overview,
                )]
            }
            "configuration:tab:connection" => {
                vec![GuiShellAction::SelectConfigurationTab(
                    GuiConfigurationTab::Connection,
                )]
            }
            "configuration:tab:playback-search" => vec![GuiShellAction::SelectConfigurationTab(
                GuiConfigurationTab::PlaybackSearch,
            )],
            "configuration:tab:privacy-chat" => {
                vec![GuiShellAction::SelectConfigurationTab(
                    GuiConfigurationTab::PrivacyChat,
                )]
            }
            "configuration:tab:interface-system" => vec![GuiShellAction::SelectConfigurationTab(
                GuiConfigurationTab::InterfaceSystem,
            )],
            "config-command:edit-room-history" => vec![GuiShellAction::BeginRoomHistoryEdit],
            "config-command:connect" => vec![GuiShellAction::BeginSavedServerConnect],
            "config-command:disconnect" => vec![GuiShellAction::BeginSessionDisconnect],
            "config-command:save" => vec![GuiShellAction::BeginConfigurationSave],
            "config-command:reset" => vec![GuiShellAction::BeginConfigurationReset],
            "config-command:reload" => vec![GuiShellAction::BeginConfigurationReload],
            "config-command:clear-gui-data" => vec![GuiShellAction::BeginClearGuiData],
            "config-player-setup:autodetect" | "main-window:player-setup:autodetect" => {
                Self::actions_for_player_setup_autodetect()
            }
            "config-player-setup:choose-path" | "main-window:player-setup:choose-path" => {
                Self::actions_for_player_setup_choose_path(state)
            }
            "config-player-setup:retry" | "main-window:player-setup:retry" => {
                vec![GuiShellAction::RetryPlayerLaunch]
            }
            "config-stream-support:import-downloader" => {
                Self::actions_for_stream_helper_import_downloader(state)
            }
            "config-stream-support:import-js-runtime" => {
                Self::actions_for_stream_helper_import_js_runtime(state)
            }
            "config-stream-support:manage" => {
                vec![GuiShellAction::OpenModal(GuiShellModal::StreamSupport)]
            }
            "config-stream-support:install" => vec![GuiShellAction::InstallStreamHelper],
            "config-stream-support:open-location" => {
                vec![GuiShellAction::OpenStreamHelperInstallLocation]
            }
            "config-stream-support:recheck" => vec![GuiShellAction::RecheckStreamHelper],
            "config-stream-support:retry" => vec![GuiShellAction::RetryPendingStreamMediaOpen],
            "main-window:player-setup:open-settings" => vec![
                GuiShellAction::SwitchView(GuiShellView::Configuration),
                GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
            ],
            "main-window:connection:connect" => vec![GuiShellAction::BeginSavedServerConnect],
            "main-window:connection:disconnect" => {
                vec![GuiShellAction::BeginSessionDisconnect]
            }
            "main-window:control:open-url" => vec![GuiShellAction::BeginMediaUrlEdit],
            "main-window:control:play" => vec![GuiShellAction::BeginPlaybackResume],
            "main-window:control:pause" => vec![GuiShellAction::BeginPlaybackPause],
            "main-window:control:toggle-pause" => vec![GuiShellAction::BeginPlaybackPauseToggle],
            "main-window:control:seek" => vec![GuiShellAction::RequestSeekPrompt],
            "main-window:control:undo-seek" => vec![GuiShellAction::RequestPlaybackUndoSeek],
            "main-window:control:set-offset" => vec![GuiShellAction::RequestOffsetPrompt],
            "main-window:control:autoplay-threshold-down" => state
                .main_window
                .autoplay_threshold
                .checked_sub(1)
                .filter(|threshold| *threshold >= 2)
                .map(GuiShellAction::AnnounceAutoplayThreshold)
                .into_iter()
                .collect(),
            "main-window:control:autoplay-threshold-up" => {
                vec![GuiShellAction::AnnounceAutoplayThreshold(
                    state
                        .main_window
                        .autoplay_threshold
                        .saturating_add(1)
                        .min(99),
                )]
            }
            "main-window:room:set" => {
                vec![GuiShellAction::SetMainWindowRoom(
                    Self::main_window_room_draft(state),
                )]
            }
            "main-window:room:join" => {
                vec![GuiShellAction::JoinMainWindowRoom(
                    Self::main_window_room_draft(state),
                )]
            }
            "main-window:room:leave" => vec![GuiShellAction::LeaveMainWindowRoom],
            "main-window:user:add" => vec![GuiShellAction::CommitNewMainWindowUser],
            "main-window:user:toggle-ready" => {
                vec![GuiShellAction::ToggleSelectedMainWindowUserReady]
            }
            "main-window:user:toggle-controller" => {
                vec![GuiShellAction::ToggleSelectedMainWindowUserController]
            }
            "main-window:user:edit" => vec![GuiShellAction::BeginEditSelectedMainWindowUser],
            "main-window:user:remove" => vec![GuiShellAction::RemoveSelectedMainWindowUser],
            "main-window:user-edit:commit" => vec![GuiShellAction::CommitMainWindowUserEdit],
            "main-window:user-edit:cancel" => vec![GuiShellAction::CancelMainWindowUserEdit],
            "main-window:playlist:up" => vec![GuiShellAction::MoveSelectedMainWindowPlaylistUp],
            "main-window:playlist:down" => {
                vec![GuiShellAction::MoveSelectedMainWindowPlaylistDown]
            }
            "main-window:playlist:remove" => {
                vec![GuiShellAction::RemoveSelectedMainWindowPlaylist]
            }
            "main-window:playlist:add-url" => vec![GuiShellAction::BeginSharedPlaylistUrlEdit],
            "main-window:playlist:open-selected" => state
                .selected_shared_playlist_entry()
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserMediaOpen(
                        target.to_owned(),
                    )]
                })
                .unwrap_or_default(),
            "main-window:playlist:open-selected-folder" => state
                .selected_shared_playlist_entry()
                .map(|target| {
                    vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
                        target.to_owned(),
                    )]
                })
                .unwrap_or_default(),
            "main-window:playlist:trust-selected" => state
                .selected_shared_playlist_entry()
                .and_then(browser_domain_from_url)
                .map(|domain| vec![GuiShellAction::AddTrustedDomain(domain)])
                .unwrap_or_default(),
            "main-window:playlist:shuffle-remaining" => {
                vec![GuiShellAction::ShuffleRemainingSharedPlaylist]
            }
            "main-window:playlist:shuffle-entire" => {
                vec![GuiShellAction::ShuffleEntireSharedPlaylist]
            }
            "main-window:playlist:undo" => vec![GuiShellAction::UndoSharedPlaylistChange],
            "main-window:playlist:edit" => vec![GuiShellAction::BeginSharedPlaylistTextEdit],
            "main-window:playlist-edit:commit" => {
                let entries = state
                    .playlist_text_edit_session
                    .as_ref()
                    .map(|session| playlist_entries_from_multiline_text(&session.buffer))
                    .unwrap_or_default();
                vec![
                    GuiShellAction::ReplaceSharedPlaylistEntries(entries),
                    GuiShellAction::CancelSharedPlaylistTextEdit,
                ]
            }
            "main-window:playlist-edit:close" => {
                vec![GuiShellAction::CancelSharedPlaylistTextEdit]
            }
            "main-window:playlist-url-edit:commit" => {
                let entries = state
                    .playlist_url_edit_session
                    .as_ref()
                    .map(|session| playlist_entries_from_multiline_text(&session.buffer))
                    .unwrap_or_default();
                vec![
                    GuiShellAction::AppendSharedPlaylistEntries(entries),
                    GuiShellAction::CancelSharedPlaylistUrlEdit,
                ]
            }
            "main-window:playlist-url-edit:close" => {
                vec![GuiShellAction::CancelSharedPlaylistUrlEdit]
            }
            "main-window:media-url-edit:commit" => state
                .media_url_edit_session
                .as_ref()
                .and_then(|session| normalized_editable_text(&session.buffer))
                .map(|target| {
                    vec![
                        GuiShellAction::RequestMainWindowUserMediaOpen(target),
                        GuiShellAction::CancelMediaUrlEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:media-url-edit:cancel" => vec![GuiShellAction::CancelMediaUrlEdit],
            "main-window:controlled-room-create:commit" => state
                .controlled_room_create_session
                .as_ref()
                .and_then(|session| {
                    let room_name =
                        controlled_room_base_name_legacy_compatible(&session.room_buffer);
                    nonempty_room_name_text(&room_name)
                })
                .map(|room| {
                    vec![
                        GuiShellAction::RequestControllerAuth {
                            room,
                            password: generate_room_password_legacy_compatible(),
                        },
                        GuiShellAction::CancelCreateControlledRoomEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:controlled-room-create:cancel" => {
                vec![GuiShellAction::CancelCreateControlledRoomEdit]
            }
            "main-window:controller-auth:commit" => state
                .controller_auth_edit_session
                .as_ref()
                .filter(|session| normalized_editable_text(&session.password_buffer).is_some())
                .map(|session| {
                    vec![
                        GuiShellAction::RequestControllerAuth {
                            room: session.room_name.clone(),
                            password: session.password_buffer.clone(),
                        },
                        GuiShellAction::CancelControllerAuthEdit,
                    ]
                })
                .unwrap_or_default(),
            "main-window:controller-auth:cancel" => {
                vec![GuiShellAction::CancelControllerAuthEdit]
            }
            "main-window:control:set-ready" => {
                let local_user_ready = state.displayed_local_main_window_user_ready();
                vec![if local_user_ready {
                    GuiShellAction::AnnounceLocalUserNotReady
                } else {
                    GuiShellAction::AnnounceLocalUserReady
                }]
            }
            "public-servers:command:connect" => {
                vec![GuiShellAction::BeginSelectedPublicServerConnect]
            }
            "public-servers:command:refresh" => vec![GuiShellAction::BeginPublicServerRefresh],
            "public-servers:command:add-custom" => vec![GuiShellAction::BeginAddPublicServer],
            "public-servers:command:edit" => vec![GuiShellAction::BeginEditSelectedPublicServer],
            "public-servers:command:remove" => vec![GuiShellAction::RemoveSelectedPublicServer],
            "public-servers:edit:commit" => vec![GuiShellAction::CommitPublicServerEdit],
            "public-servers:edit:cancel" => vec![GuiShellAction::CancelPublicServerEdit],
            "media-search:directory:up" => {
                vec![GuiShellAction::MoveSelectedMediaSearchDirectoryUp]
            }
            "media-search:directory:down" => {
                vec![GuiShellAction::MoveSelectedMediaSearchDirectoryDown]
            }
            "media-search:directory:remove" => {
                vec![GuiShellAction::RemoveSelectedMediaSearchDirectory]
            }
            "media-search:command:search" => vec![GuiShellAction::BeginMissingMediaSearch],
            "room-history:edit:commit" => vec![GuiShellAction::CommitRoomHistoryEdit],
            "room-history:edit:cancel" => vec![GuiShellAction::CancelRoomHistoryEdit],
            "shell:modal:close" => vec![GuiShellAction::CloseModal],
            "shell:modal:update:dismiss" => vec![GuiShellAction::DismissUpdateNotice],
            "shell:modal:update:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:update:check-again" => {
                vec![GuiShellAction::BeginUpdateCheck {
                    user_initiated: true,
                }]
            }
            "shell:modal:player-setup:autodetect" => {
                let mut actions = Self::actions_for_player_setup_autodetect();
                actions.push(GuiShellAction::CloseModal);
                actions
            }
            "shell:modal:player-setup:choose-path" => {
                let mut actions = Self::actions_for_player_setup_choose_path(state);
                if !actions.is_empty() {
                    actions.push(GuiShellAction::CloseModal);
                }
                actions
            }
            "shell:modal:player-setup:retry" => vec![GuiShellAction::RetryPlayerLaunch],
            "shell:modal:player-setup:open-settings" => vec![
                GuiShellAction::CloseModal,
                GuiShellAction::SwitchView(GuiShellView::Configuration),
                GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
            ],
            "shell:modal:stream-support:install" => vec![GuiShellAction::InstallStreamHelper],
            "shell:modal:stream-support:import-downloader" => {
                Self::actions_for_stream_helper_import_downloader(state)
            }
            "shell:modal:stream-support:import-js-runtime" => {
                Self::actions_for_stream_helper_import_js_runtime(state)
            }
            "shell:modal:stream-support:open-location" => {
                vec![GuiShellAction::OpenStreamHelperInstallLocation]
            }
            "shell:modal:stream-support:recheck" => vec![GuiShellAction::RecheckStreamHelper],
            "shell:modal:stream-support:retry" => {
                vec![GuiShellAction::RetryPendingStreamMediaOpen]
            }
            "shell:modal:stream-support:open-settings" => vec![
                GuiShellAction::CloseModal,
                GuiShellAction::SwitchView(GuiShellView::Configuration),
                GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
            ],
            "shell:modal:tls:trust" => vec![GuiShellAction::TrustTlsCertificatePrompt],
            "shell:modal:tls:reject" => vec![GuiShellAction::RejectTlsCertificatePrompt],
            "shell:modal:tls:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:about:help" => vec![GuiShellAction::AnnounceHelpRequested],
            "shell:modal:about:update" => {
                vec![GuiShellAction::BeginUpdateCheck {
                    user_initiated: true,
                }]
            }
            _ => {
                if let Some((section_index, action_index)) = Self::menu_action_identity(node) {
                    vec![
                        GuiShellAction::SelectMenuAction {
                            section_index,
                            action_index,
                        },
                        GuiShellAction::TriggerSelectedMenuAction,
                    ]
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub(super) fn actions_for_clicked_button(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Vec<GuiShellAction> {
        match node.id.as_str() {
            "media-search:command:browse" => Self::actions_for_media_search_browse_click(state),
            _ => Self::actions_for_button_node(state, node),
        }
    }

    fn actions_for_media_search_browse_click(
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        Self::pick_media_search_directories(state)
            .into_iter()
            .flatten()
            .map(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed)
            .collect()
    }

    fn actions_for_player_setup_autodetect() -> Vec<GuiShellAction> {
        let Some(path) = mpv_launch::autodetect_mpv_player_path_legacy_compatible() else {
            let message =
                "Automatic mpv detection did not find an executable. Choose mpv.exe manually."
                    .to_owned();
            return vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(message),
            ];
        };
        let message = format!("Player Path updated to detected mpv binary: {path}");
        vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Player Path",
                value: path,
            },
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
            GuiShellAction::RetryPlayerLaunch,
        ]
    }

    fn actions_for_player_setup_choose_path(
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let Some(path) = Self::pick_player_executable(state) else {
            return Vec::new();
        };
        let message = format!("Player Path updated to: {path}");
        vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Player Path",
                value: path,
            },
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
            GuiShellAction::RetryPlayerLaunch,
        ]
    }

    fn actions_for_stream_helper_import_downloader(
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let Some(path) = Self::pick_stream_helper_downloader_executable(state) else {
            return Vec::new();
        };
        vec![GuiShellAction::IntegrateStreamHelperDownloader(path)]
    }

    fn actions_for_stream_helper_import_js_runtime(
        state: &SyncplayGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let Some(path) = Self::pick_stream_helper_js_runtime_executable(state) else {
            return Vec::new();
        };
        vec![GuiShellAction::IntegrateStreamHelperJsRuntime(path)]
    }

    pub(super) fn is_open_media_file_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "File", "Open Media File")
    }

    pub(super) fn is_exit_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "File", "Exit")
    }

    pub(super) fn direct_menu_actions(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<GuiShellAction>> {
        let actions = if Self::matches_menu_action(state, node, "Playback", "Seek") {
            vec![GuiShellAction::RequestSeekPrompt]
        } else if Self::matches_menu_action(state, node, "Playback", "Undo Seek") {
            vec![GuiShellAction::RequestPlaybackUndoSeek]
        } else if Self::matches_menu_action(state, node, "Advanced", "Set Offset") {
            vec![GuiShellAction::RequestOffsetPrompt]
        } else {
            return None;
        };
        Some(actions)
    }

    pub(super) fn is_seek_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> bool {
        Self::matches_menu_action(state, node, "Playback", "Seek")
    }

    fn matches_menu_action(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        section_title: &str,
        action_label: &str,
    ) -> bool {
        let Some((section_index, action_index)) = Self::menu_action_identity(node) else {
            return false;
        };
        let Some(section) = state.menus.sections.get(section_index) else {
            return false;
        };
        let Some(action) = section.actions.get(action_index) else {
            return false;
        };
        section.title == section_title && action.label == action_label
    }

    pub(super) fn action_for_list_item_node(node: &GuiWidgetNode) -> Option<GuiShellAction> {
        Self::parse_index_suffix(&node.id, "main-window:user:")
            .map(GuiShellAction::SelectMainWindowUser)
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "main-window:playlist:")
                    .map(GuiShellAction::SelectMainWindowPlaylist)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "public-servers:row:")
                    .map(GuiShellAction::SelectPublicServer)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "media-search:directory:")
                    .map(GuiShellAction::SelectMediaSearchDirectory)
            })
            .or_else(|| {
                Self::parse_index_suffix(&node.id, "shell:notification:")
                    .map(GuiShellAction::DismissTransientNotification)
            })
    }

    pub(super) fn action_for_checkbox_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        value: bool,
    ) -> Option<GuiShellAction> {
        if node.id == "main-window:control:autoplay-toggle" {
            return Some(GuiShellAction::AnnounceAutoplayState(value));
        }
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Checkbox {
            return None;
        }
        Some(GuiShellAction::EditConfigurationBool {
            section,
            label,
            value,
        })
    }

    pub(super) fn actions_for_text_input_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
        value: &str,
        changed: bool,
        submitted: bool,
    ) -> Option<Vec<GuiShellAction>> {
        if node.id == "main-window:chat-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
                    GuiDraftRuntimeSnapshot {
                        outgoing_chat_message: normalized_editable_text(value),
                    },
                ));
            }
            if submitted {
                actions.push(GuiShellAction::BeginLocalChatSend(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:room-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::EditConfigurationText {
                    section: "Connection",
                    label: "Room",
                    value: value.to_owned(),
                });
            }
            if submitted && nonempty_room_name_text(value).is_some() {
                actions.push(GuiShellAction::JoinMainWindowRoom(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:user:new" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateNewMainWindowUserDraft(
                    value.to_owned(),
                ));
            }
            if submitted && normalized_editable_text(value).is_some() {
                actions.push(GuiShellAction::CommitNewMainWindowUser);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "room-history:edit:entries" {
            return changed.then(|| vec![GuiShellAction::UpdateRoomHistoryEdit(value.to_owned())]);
        }

        if node.id == "main-window:playlist-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:playlist-url-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:media-url-edit:text" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateMediaUrlEdit(value.to_owned()));
            }
            if submitted && let Some(target) = normalized_editable_text(value) {
                actions.push(GuiShellAction::RequestMainWindowUserMediaOpen(target));
                actions.push(GuiShellAction::CancelMediaUrlEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controlled-room-create:room" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateCreateControlledRoomEdit(
                    value.to_owned(),
                ));
            }
            if submitted {
                let room_name = controlled_room_base_name_legacy_compatible(value);
                if let Some(room_name) = nonempty_room_name_text(&room_name) {
                    actions.push(GuiShellAction::RequestControllerAuth {
                        room: room_name,
                        password: generate_room_password_legacy_compatible(),
                    });
                    actions.push(GuiShellAction::CancelCreateControlledRoomEdit);
                }
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controller-auth:password" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateControllerAuthPasswordEdit(
                    value.to_owned(),
                ));
            }
            if submitted
                && let Some(session) = state.controller_auth_edit_session.as_ref()
                && normalized_editable_text(value).is_some()
            {
                actions.push(GuiShellAction::RequestControllerAuth {
                    room: session.room_name.clone(),
                    password: value.to_owned(),
                });
                actions.push(GuiShellAction::CancelControllerAuthEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if let Some((section, label, kind)) = Self::configuration_control_identity(state, node) {
            if matches!(
                kind,
                GuiDialogControlKind::TextInput
                    | GuiDialogControlKind::TextArea
                    | GuiDialogControlKind::PasswordInput
                    | GuiDialogControlKind::NumericInput
                    | GuiDialogControlKind::Select
            ) && changed
            {
                return Some(vec![GuiShellAction::EditConfigurationText {
                    section,
                    label,
                    value: value.to_owned(),
                }]);
            }
            return None;
        }

        let mut actions = Vec::new();
        match node.id.as_str() {
            "public-servers:edit:label" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditLabel(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "public-servers:edit:address" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditAddress(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "main-window:user-edit:username" => {
                if changed {
                    actions.push(GuiShellAction::UpdateMainWindowUserEdit(value.to_owned()));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitMainWindowUserEdit);
                }
            }
            _ => {}
        }
        (!actions.is_empty()).then_some(actions)
    }

    fn configuration_control_identity(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<(&'static str, &'static str, GuiDialogControlKind)> {
        let identity = node.id.strip_prefix("config:")?;
        let (section, label) = identity.split_once(':')?;
        state.configuration.control_identity(section, label)
    }

    fn menu_action_identity(node: &GuiWidgetNode) -> Option<(usize, usize)> {
        let identity = node.id.strip_prefix("menus:action:")?;
        let (section_index, action_index) = identity.split_once(':')?;
        Some((section_index.parse().ok()?, action_index.parse().ok()?))
    }

    pub(super) fn configuration_select_options_for_node(
        state: &SyncplayGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<String>> {
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Select {
            return None;
        }
        Some(match (section, label) {
            ("Readiness", "Unpause Action") => [
                "IfAlreadyReady",
                "IfOthersReady",
                "IfMinUsersReady",
                "Always",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            ("Readiness", "Autoplay Min Users") => {
                let mut options = ["app-default", "0", "1", "2", "3", "4", "5"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if let Some(value) = node.value.as_ref()
                    && !value.is_empty()
                    && !options.iter().any(|option| option == value)
                {
                    options.push(value.clone());
                }
                options
            }
            ("Privacy", "Filename Privacy") | ("Privacy", "Filesize Privacy") => {
                ["SendRaw", "SendHashed", "DoNotSend"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            }
            ("Chat", "Input Position") => ["Top", "Middle", "Bottom"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ("Chat", "Output Mode") => ["Chatroom", "Scrolling"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ("System", "Language") => SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY
                .split('/')
                .map(str::to_owned)
                .collect(),
            _ => return None,
        })
    }

    fn main_window_room_draft(state: &SyncplayGuiShellAppState) -> String {
        state
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default()
            .to_owned()
    }

    fn parse_index_suffix(id: &str, prefix: &str) -> Option<usize> {
        id.strip_prefix(prefix)?.parse().ok()
    }

    fn main_window_browser_room_action_index(id: &str, action: &str) -> Option<usize> {
        let identity = id.strip_prefix("main-window:room-group:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }

    fn main_window_browser_user_action_index(id: &str, action: &str) -> Option<usize> {
        let identity = id.strip_prefix("main-window:user:")?;
        let (index, suffix) = identity.split_once(':')?;
        (suffix == action).then(|| index.parse().ok()).flatten()
    }
}
