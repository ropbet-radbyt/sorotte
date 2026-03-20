use syncplay_client_app::app_boundary::{
    commands::{
        PlannedLocalRuntimeAction, localized_current_offset_message_legacy_compatible,
        plan_local_offset_runtime_dispatch_legacy_compatible,
    },
    persistence::{
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    },
};
use syncplay_player_api::PlayerAdapter;

use super::super::runtime_bridge::{GuiPendingCompletionRequest, GuiRuntimeRequest};
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiSessionTransportDriver, GuiTcpSessionTransportDriver,
};
use super::super::shell_state::{
    GuiShellAction, GuiTransientNotificationLevel, SyncplayGuiShellAppState,
};
use super::super::startup_support::env_trimmed;
use super::super::support::normalized_editable_text;
use super::GuiPersistedConfigRuntimeOwner;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_runtime_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> bool {
        match request {
            GuiRuntimeRequest::OpenMediaFiles {
                paths,
                load_into_shared_playlist: true,
            } => {
                self.open_media_files_through_shared_playlist_runtime(
                    handle,
                    projected_state,
                    paths,
                );
            }
            GuiRuntimeRequest::OpenMediaFiles {
                paths,
                load_into_shared_playlist: false,
            } => {
                if paths.is_empty() {
                    return false;
                }
                if projected_state.playlist_backed_media_opens_preferred() {
                    self.open_media_files_through_shared_playlist_runtime(
                        handle,
                        projected_state,
                        paths,
                    );
                    return true;
                }
                self.ensure_configured_player_attached();
                if self.player.is_some() {
                    self.open_media_files_through_attached_player(handle, paths);
                } else {
                    Self::push_runtime_unavailable(
                        handle,
                        self.open_media_unavailable_message(&paths),
                    );
                }
            }
            GuiRuntimeRequest::OpenMainWindowUserMedia(target) => {
                self.open_main_window_user_media_runtime(handle, projected_state, target);
            }
            GuiRuntimeRequest::OpenMainWindowUserContainingFolder(target) => {
                self.open_main_window_user_containing_folder_runtime(
                    handle,
                    projected_state,
                    target,
                );
            }
            GuiRuntimeRequest::UndoSeek => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    Self::push_runtime_unavailable(
                        handle,
                        "Playback undo seek requires a playback runtime connection.".to_owned(),
                    );
                    return false;
                }
                match self.undo_seek_target_position_from_detached_session(projected_state) {
                        Ok(Some(target_position_seconds)) => {
                            let (player_name, undo_result) = {
                                let player = self.player.as_mut().expect("player should exist");
                                (player.name(), player.set_position(target_position_seconds))
                            };
                            match undo_result {
                                Ok(()) => {
                                    self.player_position_seconds = Some(target_position_seconds);
                                    self.refresh_player_state();
                                    Self::push_player_success(
                                        handle,
                                        format!(
                                            "Undo seek applied via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                        ),
                                    );
                                }
                                Err(error) => Self::push_player_error(
                                    handle,
                                    format!(
                                        "Playback undo seek through the attached {player_name} player failed: {error}"
                                    ),
                                ),
                            }
                        }
                        Ok(None) => Self::push_player_error(
                            handle,
                            "Playback undo seek is unavailable because no earlier seek target is recorded."
                                .to_owned(),
                        ),
                        Err(error) => Self::push_player_error(handle, error),
                    }
            }
            GuiRuntimeRequest::SetOffset(command) => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_none() {
                    Self::push_runtime_unavailable(
                        handle,
                        "Playback offset changes require a playback runtime connection.".to_owned(),
                    );
                    return false;
                }
                let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
                let dispatch = plan_local_offset_runtime_dispatch_legacy_compatible(
                    self.user_offset_seconds,
                    previous_position_seconds,
                    &command,
                    None,
                );
                let Some(PlannedLocalRuntimeAction::SeekToPosition(target_position_seconds)) =
                    dispatch.action
                else {
                    return false;
                };
                let (player_name, offset_result) = {
                    let player = self.player.as_mut().expect("player should exist");
                    (player.name(), player.set_position(target_position_seconds))
                };
                match offset_result {
                    Ok(()) => {
                        self.player_position_seconds = Some(target_position_seconds);
                        self.user_offset_seconds = dispatch
                            .updated_user_offset_seconds
                            .unwrap_or(self.user_offset_seconds);
                        self.refresh_player_state();
                        if let Err(error) = self.sync_manual_seek_into_detached_session(
                            projected_state,
                            previous_position_seconds,
                            target_position_seconds,
                        ) {
                            Self::push_player_error(handle, error);
                        }
                        let message = dispatch.line_to_emit.unwrap_or_else(|| {
                            localized_current_offset_message_legacy_compatible(
                                self.user_offset_seconds,
                                None,
                            )
                        });
                        Self::push_player_success(
                            handle,
                            format!("{message} Applied via the attached {player_name} player."),
                        );
                    }
                    Err(error) => Self::push_player_error(
                        handle,
                        format!(
                            "Playback offset change through the attached {player_name} player failed: {error}"
                        ),
                    ),
                }
            }
            GuiRuntimeRequest::SetAutoplayEnabled(enabled) => {
                if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
                    Self::push_player_error(handle, error);
                    return false;
                }
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.set_autoplay_enabled(enabled)
                {
                    Self::push_player_error(handle, error);
                }
            }
            GuiRuntimeRequest::SetAutoplayThreshold(threshold) => {
                if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
                    Self::push_player_error(handle, error);
                    return false;
                }
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.set_autoplay_threshold(threshold)
                {
                    Self::push_player_error(handle, error);
                }
            }
            GuiRuntimeRequest::SeekOffset(offset_seconds) => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_some() {
                    let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
                    let target_position_seconds =
                        (previous_position_seconds + offset_seconds).max(0.0);
                    let (player_name, seek_result) = {
                        let player = self.player.as_mut().expect("player should exist");
                        (player.name(), player.set_position(target_position_seconds))
                    };
                    match seek_result {
                        Ok(()) => {
                            self.player_position_seconds = Some(target_position_seconds);
                            self.refresh_player_state();
                            if let Err(error) = self.sync_manual_seek_into_detached_session(
                                projected_state,
                                previous_position_seconds,
                                target_position_seconds,
                            ) {
                                Self::push_player_error(handle, error);
                            }
                            Self::push_player_success(
                                handle,
                                format!(
                                    "Applied a {offset_seconds} second seek via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                ),
                            );
                        }
                        Err(error) => {
                            Self::push_player_error(
                                handle,
                                format!(
                                    "Playback seek through the attached {player_name} player failed: {error}"
                                ),
                            );
                        }
                    }
                } else {
                    Self::push_runtime_unavailable(
                        handle,
                        self.seek_unavailable_message(offset_seconds),
                    );
                }
            }
            GuiRuntimeRequest::SeekToPosition(target_position_seconds) => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_some() {
                    let previous_position_seconds = self.player_position_seconds.unwrap_or(0.0);
                    let target_position_seconds = target_position_seconds.max(0.0);
                    let (player_name, seek_result) = {
                        let player = self.player.as_mut().expect("player should exist");
                        (player.name(), player.set_position(target_position_seconds))
                    };
                    match seek_result {
                        Ok(()) => {
                            self.player_position_seconds = Some(target_position_seconds);
                            self.refresh_player_state();
                            if let Err(error) = self.sync_manual_seek_into_detached_session(
                                projected_state,
                                previous_position_seconds,
                                target_position_seconds,
                            ) {
                                Self::push_player_error(handle, error);
                            }
                            Self::push_player_success(
                                handle,
                                format!(
                                    "Applied an absolute seek via the attached {player_name} player (target {target_position_seconds:.3} seconds)."
                                ),
                            );
                        }
                        Err(error) => {
                            Self::push_player_error(
                                handle,
                                format!(
                                    "Playback seek through the attached {player_name} player failed: {error}"
                                ),
                            );
                        }
                    }
                } else {
                    Self::push_runtime_unavailable(
                        handle,
                        "Playback absolute seek requires a playback runtime connection.".to_owned(),
                    );
                }
            }
            GuiRuntimeRequest::SetRoom(room) => {
                self.request_room_join_runtime(handle, projected_state, room);
            }
            GuiRuntimeRequest::ReturnToDefaultRoom => {
                self.request_room_leave_runtime(handle, projected_state);
            }
            GuiRuntimeRequest::SetLocalReady(ready) => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.set_local_ready(ready)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::SetReadyForUser { username, ready } => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.set_user_ready(username, ready)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::RequestControllerAuth { room, password } => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.request_controller_auth(room, password)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::QueuePlaylistEntry {
                entry,
                select_after_queue,
            } => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.queue_playlist_entry(entry, select_after_queue)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::SetPlaylistIndex(index) => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.set_playlist_index(index)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::DeletePlaylistIndex(index) => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.delete_playlist_index(index)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::UndoPlaylistChange => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.undo_playlist_change()
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::ShuffleRemainingPlaylist => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.shuffle_remaining_playlist()
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::ShuffleEntirePlaylist => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.shuffle_entire_playlist()
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::AdvancePlaylistIndex => {
                if let Some(session) = self.session.as_mut() {
                    if let Err(error) = session.advance_playlist_index() {
                        handle.push_action(GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: error,
                        });
                    }
                } else {
                    Self::push_runtime_unavailable(
                        handle,
                        "Advancing the shared playlist requires an active session runtime."
                            .to_owned(),
                    );
                }
            }
            GuiRuntimeRequest::ReplacePlaylist {
                files,
                selected_index,
            } => {
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.replace_playlist(files, selected_index)
                {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
            GuiRuntimeRequest::SendChatMessage(message) => {
                if let Some(session) = self.session.as_mut() {
                    match session.send_chat_message(message.clone()) {
                        Ok(()) => {
                            let sender = projected_state
                                .main_window
                                .users
                                .iter()
                                .find(|user| user.is_self)
                                .map(|user| user.username.clone())
                                .unwrap_or_else(|| "You".to_owned());
                            Self::push_actions_and_project(
                                handle,
                                projected_state,
                                vec![
                                    GuiShellAction::PushChatMessage { sender, message },
                                    GuiShellAction::PushTransientNotification {
                                        level: GuiTransientNotificationLevel::Success,
                                        message: "Chat sent.".to_owned(),
                                    },
                                ],
                            );
                        }
                        Err(error) => Self::push_runtime_unavailable(
                            handle,
                            format!(
                                "Chat sending through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
                } else {
                    Self::push_runtime_unavailable(handle, self.send_chat_unavailable_message());
                }
            }
            GuiRuntimeRequest::TogglePlaybackPause => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_some() {
                    let target_paused = !projected_state.main_window.playback_paused;
                    let previous_paused = projected_state.main_window.playback_paused;
                    let (player_name, toggle_result) = {
                        let player = self.player.as_mut().expect("player should exist");
                        (player.name(), player.set_paused(target_paused))
                    };
                    match toggle_result {
                        Ok(()) => {
                            self.player_paused = Some(target_paused);
                            self.refresh_player_state();
                            if let Err(error) = self.sync_playback_pause_into_detached_session(
                                projected_state,
                                previous_paused,
                                target_paused,
                            ) {
                                Self::push_player_error(handle, error);
                            }
                            Self::push_actions_and_project(
                                handle,
                                projected_state,
                                vec![if target_paused {
                                    GuiShellAction::AnnouncePlaybackPaused
                                } else {
                                    GuiShellAction::AnnouncePlaybackResumed
                                }],
                            );
                        }
                        Err(error) => Self::push_player_error(
                            handle,
                            format!(
                                "Playback pause toggle through the attached {player_name} player failed: {error}"
                            ),
                        ),
                    }
                } else {
                    Self::push_runtime_unavailable(handle, self.toggle_pause_unavailable_message());
                }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::TogglePlaybackPause,
            ) => {
                self.refresh_player_state();
                self.ensure_configured_player_attached();
                if self.player.is_some() {
                    let target_paused = !projected_state.main_window.playback_paused;
                    let previous_paused = projected_state.main_window.playback_paused;
                    let (player_name, toggle_result) = {
                        let player = self.player.as_mut().expect("player should exist");
                        (player.name(), player.set_paused(target_paused))
                    };
                    let actions = match toggle_result {
                        Ok(()) => {
                            self.player_paused = Some(target_paused);
                            self.refresh_player_state();
                            if let Err(error) = self.sync_playback_pause_into_detached_session(
                                projected_state,
                                previous_paused,
                                target_paused,
                            ) {
                                Self::push_player_error(handle, error);
                            }
                            vec![GuiShellAction::CompletePlaybackPauseToggle]
                        }
                        Err(error) => vec![
                            GuiShellAction::CancelPlaybackPauseToggle,
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: format!(
                                    "Playback pause toggle through the attached {player_name} player failed: {error}"
                                ),
                            },
                        ],
                    };
                    Self::push_actions_and_project(handle, projected_state, actions);
                } else {
                    let actions = vec![
                        GuiShellAction::CancelPlaybackPauseToggle,
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: self.toggle_pause_unavailable_message(),
                        },
                    ];
                    Self::push_actions_and_project(handle, projected_state, actions);
                }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ConnectSavedServer,
            ) => {
                self.complete_saved_server_connect_runtime(handle, projected_state, true);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::DisconnectSession,
            ) => {
                self.complete_session_disconnect_runtime(handle, projected_state);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ConnectPublicServer,
            ) => {
                let selected_server = projected_state
                    .selected_public_server_index()
                    .and_then(|index| projected_state.public_servers.servers.get(index))
                    .map(|row| (row.label.clone(), row.address.clone()));
                let replacement_transport_driver = selected_server
                    .as_ref()
                    .map(|(_label, address)| {
                        GuiTcpSessionTransportDriver::connect_from_host_arg(address)
                            .map(|driver| Box::new(driver) as Box<dyn GuiSessionTransportDriver>)
                    })
                    .transpose();
                let replacement_transport_driver = match replacement_transport_driver {
                    Ok(driver) => driver,
                    Err(error) => {
                        self.clear_pending_operation_with_runtime_error(
                                handle,
                                projected_state,
                                format!(
                                    "Public server connect through the attached session runtime failed: {error}"
                                ),
                            );
                        return false;
                    }
                };
                if let Err(error) = self.ensure_detached_client_core_chat_session(projected_state) {
                    self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            format!(
                                "Public server connect through the attached session runtime failed: {error}"
                            ),
                        );
                    return false;
                }
                let Some(session) = self.session.as_mut() else {
                    self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            "Public server connect could not bootstrap a detached client-core session runtime."
                                .to_owned(),
                        );
                    return false;
                };
                match session.connect_public_server(selected_server) {
                        Ok(()) => {
                            self.session_projects_to_shell = true;
                            if let Some(driver) = replacement_transport_driver {
                                if let Some(session_transport) = self.session_transport.as_ref() {
                                    session_transport.clear_protocol_lines();
                                }
                                self.session_transport_driver = Some(driver);
                            }
                            Self::push_actions_and_project(
                                handle,
                                projected_state,
                                vec![GuiShellAction::CompleteSelectedPublicServerConnect],
                            )
                        }
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            format!(
                                "Public server connect through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::RefreshPublicServers(requested_servers),
            ) => {
                let current_servers = projected_state
                    .public_servers
                    .servers
                    .iter()
                    .map(|row| (row.label.clone(), row.address.clone()))
                    .collect();
                let language = Some(projected_state.runtime_language_tag_legacy_compatible());
                let refresh_result = if let Some(session) = self.session.as_mut() {
                    session.refresh_public_servers(current_servers, language)
                } else if !requested_servers.is_empty() {
                    Ok(
                        GuiClientCoreChatSessionRuntimeAdapter::normalize_public_server_rows(
                            requested_servers,
                        ),
                    )
                } else {
                    Self::refresh_public_servers_without_session(current_servers, language)
                };
                match refresh_result {
                        Ok(servers) => Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::CompletePublicServerRefresh(servers)],
                        ),
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            format!(
                                "Public server refresh through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SearchMissingMedia,
            ) => {
                let directories = projected_state
                    .media_search
                    .directories
                    .iter()
                    .map(|row| row.path.clone())
                    .collect();
                let search_result = if let Some(session) = self.session.as_mut() {
                    session.search_missing_media(directories)
                } else {
                    self.search_missing_media_without_session(projected_state, directories)
                };
                match search_result {
                        Ok(found_path) => {
                            let found_path =
                                found_path.and_then(|path| normalized_editable_text(&path));
                            self.ensure_configured_player_attached();
                            match found_path {
                                Some(path) if self.player.is_some() => {
                                    self.clear_pending_operation_runtime_state(
                                        handle,
                                        projected_state,
                                    );
                                    if self.current_player_matches_media_target(&path) {
                                        let player_name = self
                                            .player
                                            .as_ref()
                                            .map(|player| player.name())
                                            .unwrap_or("player");
                                        Self::push_player_success(
                                            handle,
                                            format!(
                                                "Opened media file through the attached {player_name} player: {path}."
                                            ),
                                        );
                                    } else {
                                        self.open_media_files_through_attached_player(
                                            handle,
                                            vec![path],
                                        );
                                    }
                                }
                                found_path => Self::push_actions_and_project(
                                    handle,
                                    projected_state,
                                    vec![GuiShellAction::CompleteMissingMediaSearch(found_path)],
                                ),
                            }
                        }
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            format!(
                                "Missing-media search through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SendChatMessage(message),
            ) => {
                if let Some(session) = self.session.as_mut() {
                    match session.send_chat_message(message) {
                        Ok(()) => Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::CompleteLocalChatSend],
                        ),
                        Err(error) => self.clear_pending_operation_with_runtime_error(
                            handle,
                            projected_state,
                            format!(
                                "Chat sending through the attached session runtime failed: {error}"
                            ),
                        ),
                    }
                } else {
                    self.clear_pending_operation_with_runtime_error(
                        handle,
                        projected_state,
                        self.send_chat_unavailable_message(),
                    );
                }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SaveConfiguration(settings),
            ) => {
                let Some(path) = self.config_path.as_ref() else {
                    self.sync_player_from_lookup_and_settings(&env_trimmed, Some(&settings), true);
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::CompleteConfigurationSave(settings)],
                    );
                    return false;
                };
                match upsert_syncplay_ini_stored_client_settings_mvp_at_path(path, &settings) {
                    Ok(()) => {
                        self.sync_player_from_lookup_and_settings(
                            &env_trimmed,
                            Some(&settings),
                            true,
                        );
                        Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::CompleteConfigurationSave(settings)],
                        );
                    }
                    Err(error) => Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![
                            GuiShellAction::CancelConfigurationSave,
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: format!("Configuration save failed: {error}"),
                            },
                        ],
                    ),
                }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ResetConfiguration(settings),
            ) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CompleteConfigurationReset(settings)],
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ReloadConfiguration(fallback_settings),
            ) => {
                let Some(path) = self.config_path.as_ref() else {
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::CompleteConfigurationReload(
                            fallback_settings,
                        )],
                    );
                    return false;
                };
                match load_syncplay_ini_stored_client_settings_mvp_from_path(path) {
                    Ok(Some(settings)) => {
                        self.sync_player_from_lookup_and_settings(
                            &env_trimmed,
                            Some(&settings),
                            true,
                        );
                        Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::CompleteConfigurationReload(settings)],
                        );
                    }
                    Ok(None) => {
                        self.sync_player_from_lookup_and_settings(
                            &env_trimmed,
                            Some(&fallback_settings),
                            true,
                        );
                        Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::CompleteConfigurationReload(
                                fallback_settings,
                            )],
                        );
                    }
                    Err(error) => Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![
                            GuiShellAction::CancelConfigurationReload,
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: format!("Configuration reload failed: {error}"),
                            },
                        ],
                    ),
                }
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ClearGuiData,
            ) => match self.clear_gui_data() {
                Ok(()) => {
                    self.sync_player_from_lookup_and_settings(&env_trimmed, None, true);
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::CompleteClearGuiData],
                    )
                }
                Err(error) => Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::CancelClearGuiData,
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: format!("Clear GUI data failed: {error}"),
                        },
                    ],
                ),
            },
            GuiRuntimeRequest::CancelPendingOperation(_kind) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::CancelPendingOperation],
                );
            }
        }

        true
    }
}
