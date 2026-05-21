use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn actions_for_button_node(
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
        if let Some(index) = Self::main_window_playlist_row_action_index(&node.id, "remove") {
            return if index < state.main_window.playlist.len() {
                vec![
                    GuiShellAction::SelectMainWindowPlaylist(index),
                    GuiShellAction::RemoveSelectedMainWindowPlaylist,
                ]
            } else {
                Vec::new()
            };
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
            "configuration:alert:close" => vec![GuiShellAction::DismissSetupAlert],
            "configuration:alert:fix-player-path" => vec![
                GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
                GuiShellAction::FocusConfigurationControl {
                    section: "Connection",
                    label: "Player Path",
                },
                GuiShellAction::BeginConfigurationTextEdit {
                    section: "Connection",
                    label: "Player Path",
                },
            ],
            "config-player-setup:autodetect" | "main-window:player-setup:autodetect" => {
                Self::actions_for_player_setup_autodetect()
            }
            "config-player-setup:choose-path" | "main-window:player-setup:choose-path" => {
                Self::actions_for_player_setup_choose_path(state)
            }
            "config-player-setup:retry" | "main-window:player-setup:retry" => {
                vec![GuiShellAction::RetryPlayerLaunch]
            }
            "config-stream-support:import-downloader"
            | "plugins:stream-support:import-downloader" => {
                Self::actions_for_stream_helper_import_downloader(state)
            }
            "config-stream-support:import-js-runtime"
            | "plugins:stream-support:import-js-runtime" => {
                Self::actions_for_stream_helper_import_js_runtime(state)
            }
            "config-stream-support:manage" => {
                vec![GuiShellAction::OpenModal(GuiShellModal::StreamSupport)]
            }
            "config-stream-support:install"
            | "plugins:stream-support:install"
            | "plugins:stream-support:alert:install" => vec![GuiShellAction::InstallStreamHelper],
            "config-stream-support:open-location" | "plugins:stream-support:open-location" => {
                vec![GuiShellAction::OpenStreamHelperInstallLocation]
            }
            "config-stream-support:recheck"
            | "plugins:stream-support:recheck"
            | "plugins:stream-support:alert:recheck" => vec![GuiShellAction::RecheckStreamHelper],
            "config-stream-support:retry"
            | "plugins:stream-support:retry"
            | "plugins:stream-support:alert:retry" => {
                vec![GuiShellAction::RetryPendingStreamMediaOpen]
            }
            "plugins:plex:connect" => vec![GuiShellAction::StartPlexAuth],
            "plugins:plex:poll-auth" => vec![GuiShellAction::PollPlexAuth],
            "plugins:plex:refresh-servers" => vec![GuiShellAction::RefreshPlexServers],
            "plugins:plex:enable-sync" => vec![GuiShellAction::TogglePlexSync(true)],
            "plugins:plex:disable-sync" => vec![GuiShellAction::TogglePlexSync(false)],
            "plugins:plex:disconnect" => vec![GuiShellAction::DisconnectPlex],
            id if id.starts_with("plugins:plex:server:") => id
                .strip_prefix("plugins:plex:server:")
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| state.plex.servers.get(index))
                .map(|server| {
                    vec![GuiShellAction::SelectPlexServer {
                        machine_identifier: server.machine_identifier.clone(),
                        uri: server.uri.clone(),
                    }]
                })
                .unwrap_or_default(),
            "main-window:player-setup:open-settings" => vec![
                GuiShellAction::SwitchView(GuiShellView::Setup),
                GuiShellAction::SelectConfigurationTab(GuiConfigurationTab::Connection),
            ],
            "main-window:connection:connect" => vec![GuiShellAction::BeginSavedServerConnect],
            "main-window:connection:disconnect" => {
                vec![GuiShellAction::BeginSessionDisconnect]
            }
            "main-window:room-actions:toggle" => {
                vec![GuiShellAction::ToggleMainWindowRoomChange]
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
            "main-window:room-actions:create-controlled-room" => {
                vec![GuiShellAction::BeginCreateControlledRoomEdit]
            }
            "main-window:room-actions:identify-controller" => {
                vec![GuiShellAction::BeginControllerAuthEdit]
            }
            "main-window:chat:send" => state
                .outgoing_chat_message
                .as_deref()
                .and_then(normalized_editable_text)
                .map(|message| vec![GuiShellAction::BeginLocalChatSend(message)])
                .unwrap_or_default(),
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
            "shell:modal:update:download" => vec![GuiShellAction::BeginUpdateDownload],
            "shell:modal:update:restart" => vec![GuiShellAction::BeginStagedUpdateApply],
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
                GuiShellAction::SwitchView(GuiShellView::Setup),
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
                GuiShellAction::SwitchView(GuiShellView::Plugins),
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

    pub(in crate::app) fn actions_for_clicked_button(
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
}
