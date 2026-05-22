mod pending_completions;
mod playback;
mod session_controls;
mod stream_helper;

use std::path::Path;

use sorotte_client_app::app_boundary::{
    commands::{
        PlannedLocalRuntimeAction, localized_current_offset_message_legacy_compatible,
        plan_local_offset_runtime_dispatch_legacy_compatible,
    },
    persistence::{
        load_sorotte_ini_stored_client_settings_mvp_from_path,
        upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    },
};
use sorotte_player_api::PlayerAdapter;

use super::super::remote_services;
use super::super::runtime_bridge::{GuiPendingCompletionRequest, GuiRuntimeRequest};
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiSessionTransportDriver, GuiTcpSessionTransportDriver,
};
use super::super::shell_state::{
    GuiShellAction, GuiStreamHelperHealth, GuiStreamTargetKind, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SorotteGuiShellAppState, browser_stream_target_kind,
};
use super::super::startup_support::env_trimmed;
use super::super::stream_support::{
    StreamHelperRemediationProgress, import_managed_stream_helper_downloader_with_progress,
    import_managed_stream_helper_js_runtime_with_progress,
    install_or_update_managed_stream_helper_with_progress, managed_stream_helper_bin_dir,
};
use super::super::support::normalized_editable_text;
use super::{GuiPersistedConfigRuntimeOwner, GuiUserMediaTargetResolution};

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_runtime_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> bool {
        match request {
            GuiRuntimeRequest::CheckForUpdates {
                language,
                update_channel,
                user_initiated,
            } => {
                let result = remote_services::check_for_updates(
                    Some(language.as_str()),
                    user_initiated,
                    update_channel.as_deref(),
                );
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyUpdateCheckResult(result)],
                );
            }
            GuiRuntimeRequest::DownloadUpdate(candidate) => {
                let result = remote_services::download_and_stage_update(
                    &candidate,
                    self.legacy_gui_qsettings_root().as_deref(),
                );
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyUpdateDownloadResult(result)],
                );
            }
            GuiRuntimeRequest::DownloadAndInstallUpdate(candidate) => {
                let result = remote_services::download_and_stage_update(
                    &candidate,
                    self.legacy_gui_qsettings_root().as_deref(),
                );
                let staged_update = result.staged_update.clone();
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyUpdateDownloadResult(result)],
                );
                if let Some(staged_update) = staged_update {
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::BeginStagedUpdateApply],
                    );
                    let result = remote_services::launch_staged_update(&staged_update);
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::ApplyStagedUpdateLaunchResult(result)],
                    );
                }
            }
            GuiRuntimeRequest::ApplyStagedUpdate(staged_update) => {
                let result = remote_services::launch_staged_update(&staged_update);
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::ApplyStagedUpdateLaunchResult(result)],
                );
            }
            GuiRuntimeRequest::OpenMediaFiles {
                paths,
                load_into_shared_playlist: true,
                playlist_insert_slot,
            } => {
                self.open_media_files_through_shared_playlist_runtime(
                    handle,
                    projected_state,
                    paths,
                    playlist_insert_slot,
                );
            }
            GuiRuntimeRequest::OpenMediaFiles {
                paths,
                load_into_shared_playlist: false,
                playlist_insert_slot: _,
            } => {
                if paths.is_empty() {
                    return false;
                }
                if projected_state.playlist_backed_media_opens_preferred() {
                    self.open_media_files_through_shared_playlist_runtime(
                        handle,
                        projected_state,
                        paths,
                        None,
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
            GuiRuntimeRequest::RetryPlayerLaunch => {
                return self.handle_retry_player_launch_request(handle, projected_state);
            }
            GuiRuntimeRequest::InstallStreamHelper => {
                return self.handle_install_stream_helper_request(handle, projected_state);
            }
            GuiRuntimeRequest::OpenStreamHelperInstallLocation => {
                return self
                    .handle_open_stream_helper_install_location_request(handle, projected_state);
            }
            GuiRuntimeRequest::IntegrateStreamHelperDownloader(source_path) => {
                return self.handle_integrate_stream_helper_downloader_request(
                    handle,
                    projected_state,
                    source_path,
                );
            }
            GuiRuntimeRequest::IntegrateStreamHelperJsRuntime(source_path) => {
                return self.handle_integrate_stream_helper_js_runtime_request(
                    handle,
                    projected_state,
                    source_path,
                );
            }
            GuiRuntimeRequest::RecheckStreamHelper => {
                return self.handle_recheck_stream_helper_request(handle, projected_state);
            }
            GuiRuntimeRequest::RetryPendingStreamMediaOpen => {
                return self
                    .handle_retry_pending_stream_media_open_request(handle, projected_state);
            }
            GuiRuntimeRequest::StartPlexAuth => {
                return self.handle_start_plex_auth_request(handle, projected_state);
            }
            GuiRuntimeRequest::PollPlexAuth => {
                return self.handle_poll_plex_auth_request(handle, projected_state);
            }
            GuiRuntimeRequest::RefreshPlexServers => {
                return self.handle_refresh_plex_servers_request(handle, projected_state);
            }
            GuiRuntimeRequest::SelectPlexServer {
                machine_identifier,
                uri,
            } => {
                return self.handle_select_plex_server_request(
                    handle,
                    projected_state,
                    machine_identifier,
                    uri,
                );
            }
            GuiRuntimeRequest::TogglePlexSync(enabled) => {
                return self.handle_toggle_plex_sync_request(handle, projected_state, enabled);
            }
            GuiRuntimeRequest::DisconnectPlex => {
                return self.handle_disconnect_plex_request(handle, projected_state);
            }
            GuiRuntimeRequest::UndoSeek => {
                return self.handle_undo_seek_request(handle, projected_state);
            }
            GuiRuntimeRequest::SetOffset(command) => {
                return self.handle_set_offset_request(handle, projected_state, command);
            }
            GuiRuntimeRequest::SetAutoplayEnabled(enabled) => {
                return self.handle_set_autoplay_enabled_request(handle, projected_state, enabled);
            }
            GuiRuntimeRequest::SetAutoplayThreshold(threshold) => {
                return self.handle_set_autoplay_threshold_request(
                    handle,
                    projected_state,
                    threshold,
                );
            }
            GuiRuntimeRequest::SeekOffset(offset_seconds) => {
                return self.handle_seek_offset_request(handle, projected_state, offset_seconds);
            }
            GuiRuntimeRequest::SeekToPosition(target_position_seconds) => {
                return self.handle_seek_to_position_request(
                    handle,
                    projected_state,
                    target_position_seconds,
                );
            }
            GuiRuntimeRequest::SetRoom(room) => {
                self.request_room_join_runtime(handle, projected_state, room);
            }
            GuiRuntimeRequest::ReturnToDefaultRoom => {
                self.request_room_leave_runtime(handle, projected_state);
            }
            GuiRuntimeRequest::SetLocalReady(ready) => {
                return self.handle_set_local_ready_request(handle, projected_state, ready);
            }
            GuiRuntimeRequest::SetReadyForUser { username, ready } => {
                return self.handle_set_ready_for_user_request(
                    handle,
                    projected_state,
                    username,
                    ready,
                );
            }
            GuiRuntimeRequest::RequestControllerAuth { room, password } => {
                return self.handle_request_controller_auth_request(
                    handle,
                    projected_state,
                    room,
                    password,
                );
            }
            GuiRuntimeRequest::QueuePlaylistEntry {
                entry,
                select_after_queue,
            } => {
                return self.handle_queue_playlist_entry_request(
                    handle,
                    projected_state,
                    entry,
                    select_after_queue,
                );
            }
            GuiRuntimeRequest::SetPlaylistIndex(index) => {
                return self.handle_set_playlist_index_request(handle, projected_state, index);
            }
            GuiRuntimeRequest::DeletePlaylistIndex(index) => {
                return self.handle_delete_playlist_index_request(handle, projected_state, index);
            }
            GuiRuntimeRequest::UndoPlaylistChange => {
                return self.handle_undo_playlist_change_request(handle, projected_state);
            }
            GuiRuntimeRequest::ShuffleRemainingPlaylist => {
                return self.handle_shuffle_remaining_playlist_request(handle, projected_state);
            }
            GuiRuntimeRequest::ShuffleEntirePlaylist => {
                return self.handle_shuffle_entire_playlist_request(handle, projected_state);
            }
            GuiRuntimeRequest::AdvancePlaylistIndex => {
                return self.handle_advance_playlist_index_request(handle, projected_state);
            }
            GuiRuntimeRequest::ReplacePlaylist {
                files,
                selected_index,
            } => {
                return self.handle_replace_playlist_request(
                    handle,
                    projected_state,
                    files,
                    selected_index,
                );
            }
            GuiRuntimeRequest::SendChatMessage(message) => {
                return self.handle_send_chat_message_request(handle, projected_state, message);
            }
            GuiRuntimeRequest::TogglePlaybackPause => {
                return self.handle_toggle_playback_pause_request(handle, projected_state);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::TogglePlaybackPause,
            ) => {
                return self.handle_complete_toggle_playback_pause_request(handle, projected_state);
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
                return self.handle_complete_public_server_connect_request(handle, projected_state);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::RefreshPublicServers(requested_servers),
            ) => {
                return self.handle_complete_public_server_refresh_request(
                    handle,
                    projected_state,
                    requested_servers,
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SearchMissingMedia,
            ) => {
                return self.handle_complete_missing_media_search_request(handle, projected_state);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SendChatMessage(message),
            ) => {
                return self.handle_complete_send_chat_message_request(
                    handle,
                    projected_state,
                    message,
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SaveConfiguration(settings),
            ) => {
                return self.handle_complete_configuration_save_request(
                    handle,
                    projected_state,
                    settings,
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ResetConfiguration(settings),
            ) => {
                return self.handle_complete_configuration_reset_request(
                    handle,
                    projected_state,
                    settings,
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ReloadConfiguration(fallback_settings),
            ) => {
                return self.handle_complete_configuration_reload_request(
                    handle,
                    projected_state,
                    fallback_settings,
                );
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ClearGuiData,
            ) => {
                return self.handle_complete_clear_gui_data_request(handle, projected_state);
            }
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
