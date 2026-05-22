use super::*;

impl GuiRuntimeRequest {
    pub(in crate::app) fn preview_actions_for_state(
        &self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        match self {
            Self::CheckForUpdates { user_initiated, .. } => {
                vec![GuiShellAction::BeginUpdateCheck {
                    user_initiated: *user_initiated,
                }]
            }
            Self::DownloadUpdate(_) => vec![GuiShellAction::BeginUpdateDownload],
            Self::ApplyStagedUpdate(_) => vec![GuiShellAction::BeginStagedUpdateApply],
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
                playlist_insert_slot,
            } => GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                Some(state),
                paths.clone(),
                *load_into_shared_playlist || state.playlist_backed_media_opens_preferred(),
                *playlist_insert_slot,
            ),
            Self::OpenMainWindowUserMedia(target) => {
                GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                    Some(state),
                    vec![target.clone()],
                    state.playlist_backed_media_opens_preferred(),
                    None,
                )
            }
            Self::RetryPlayerLaunch => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Retrying mpv launch with the current player settings.".to_owned(),
            }],
            Self::InstallStreamHelper => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Installing stream helper support for extractor-backed URLs.".to_owned(),
            }],
            Self::IntegrateStreamHelperDownloader(_) => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Importing yt-dlp into Sorotte's managed stream helper.".to_owned(),
                }]
            }
            Self::IntegrateStreamHelperJsRuntime(_) => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Importing Deno into Sorotte's managed stream helper.".to_owned(),
                }]
            }
            Self::OpenStreamHelperInstallLocation => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Opening Sorotte's managed stream-helper install location.".to_owned(),
                }]
            }
            Self::RecheckStreamHelper => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Rechecking stream helper support for the current URL.".to_owned(),
            }],
            Self::RetryPendingStreamMediaOpen => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Retrying the pending media URL open request.".to_owned(),
            }],
            Self::StartPlexAuth => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Starting Plex authentication.".to_owned(),
            }],
            Self::PollPlexAuth => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Checking Plex authentication status.".to_owned(),
            }],
            Self::RefreshPlexServers => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Refreshing Plex servers.".to_owned(),
            }],
            Self::SelectPlexServer { .. } => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Selecting Plex server.".to_owned(),
            }],
            Self::TogglePlexSync(enabled) => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: if *enabled {
                    "Enabling Plex watch sync.".to_owned()
                } else {
                    "Disabling Plex watch sync.".to_owned()
                },
            }],
            Self::DisconnectPlex => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Disconnecting Plex.".to_owned(),
            }],
            Self::SendChatMessage(_message) => Vec::new(),
            Self::SeekToPosition(target_position_seconds) => {
                let message = format!("Seek requested: target {target_position_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::AdvancePlaylistIndex => Vec::new(),
            Self::TogglePlaybackPause => vec![GuiShellAction::CompletePlaybackPauseToggle],
            Self::CompletePendingOperation(GuiPendingCompletionRequest::ConnectSavedServer)
                if state.pending_saved_server_connect_saves_configuration =>
            {
                vec![
                    GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
                        GuiSavedConfigurationRuntimeSnapshot {
                            settings: state.configuration.to_stored_settings(),
                        },
                    ),
                    GuiShellAction::CompleteSavedServerConnect,
                ]
            }
            _ => self.preview_actions(),
        }
    }

    pub(in crate::app) fn preview_actions(&self) -> Vec<GuiShellAction> {
        match self {
            Self::CheckForUpdates { user_initiated, .. } => {
                vec![GuiShellAction::BeginUpdateCheck {
                    user_initiated: *user_initiated,
                }]
            }
            Self::DownloadUpdate(_) => vec![GuiShellAction::BeginUpdateDownload],
            Self::ApplyStagedUpdate(_) => vec![GuiShellAction::BeginStagedUpdateApply],
            Self::OpenMediaFiles {
                paths,
                load_into_shared_playlist,
                playlist_insert_slot,
            } => GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                None,
                paths.clone(),
                *load_into_shared_playlist,
                *playlist_insert_slot,
            ),
            Self::OpenMainWindowUserMedia(target) => {
                GuiPreviewRuntimeBridge::preview_open_media_file_actions(
                    None,
                    vec![target.clone()],
                    false,
                    None,
                )
            }
            Self::OpenMainWindowUserContainingFolder(target) => {
                let message = format!("Open containing folder requested: {target}.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::RetryPlayerLaunch => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Retrying mpv launch with the current player settings.".to_owned(),
            }],
            Self::InstallStreamHelper => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Installing stream helper support for extractor-backed URLs.".to_owned(),
            }],
            Self::IntegrateStreamHelperDownloader(_) => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Importing yt-dlp into Sorotte's managed stream helper.".to_owned(),
                }]
            }
            Self::IntegrateStreamHelperJsRuntime(_) => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Importing Deno into Sorotte's managed stream helper.".to_owned(),
                }]
            }
            Self::OpenStreamHelperInstallLocation => {
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Opening Sorotte's managed stream-helper install location.".to_owned(),
                }]
            }
            Self::RecheckStreamHelper => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Rechecking stream helper support for the current URL.".to_owned(),
            }],
            Self::RetryPendingStreamMediaOpen => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Retrying the pending media URL open request.".to_owned(),
            }],
            Self::StartPlexAuth => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Starting Plex authentication.".to_owned(),
            }],
            Self::PollPlexAuth => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Checking Plex authentication status.".to_owned(),
            }],
            Self::RefreshPlexServers => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Refreshing Plex servers.".to_owned(),
            }],
            Self::SelectPlexServer { .. } => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Selecting Plex server.".to_owned(),
            }],
            Self::TogglePlexSync(enabled) => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: if *enabled {
                    "Enabling Plex watch sync.".to_owned()
                } else {
                    "Disabling Plex watch sync.".to_owned()
                },
            }],
            Self::DisconnectPlex => vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Disconnecting Plex.".to_owned(),
            }],
            Self::UndoSeek => vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Undo seek requested.".to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent("Undo seek requested.".to_owned()),
            ],
            Self::SetOffset(command) => {
                let message = format!("Offset requested: {}.", format_offset_command(command));
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetAutoplayEnabled(_)
            | Self::SetAutoplayThreshold(_)
            | Self::SetReadyForUser { .. }
            | Self::RequestControllerAuth { .. }
            | Self::QueuePlaylistEntry { .. }
            | Self::SetPlaylistIndex(_)
            | Self::AdvancePlaylistIndex
            | Self::DeletePlaylistIndex(_)
            | Self::UndoPlaylistChange
            | Self::ShuffleRemainingPlaylist
            | Self::ShuffleEntirePlaylist
            | Self::ReplacePlaylist { .. }
            | Self::SetRoom(_)
            | Self::ReturnToDefaultRoom
            | Self::SendChatMessage(_)
            | Self::TogglePlaybackPause => Vec::new(),
            Self::SeekOffset(offset_seconds) => {
                let message = format!("Seek requested: {offset_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SeekToPosition(target_position_seconds) => {
                let message = format!("Seek requested: target {target_position_seconds} seconds.");
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: message.clone(),
                    },
                    GuiShellAction::AnnounceSystemChatEvent(message),
                ]
            }
            Self::SetLocalReady(_) => Vec::new(),
            Self::CompletePendingOperation(request) => vec![request.clone().into_action()],
            Self::CancelPendingOperation(_) => vec![GuiShellAction::CancelPendingOperation],
        }
    }
}
