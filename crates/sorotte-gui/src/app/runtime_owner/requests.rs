mod media_match;
mod pending_completions;
mod playback;
mod session_controls;
mod stream_helper;

use std::path::{Path, PathBuf};

use sorotte_client_app::app_boundary::{
    commands::{
        PlannedLocalRuntimeAction, localized_current_offset_message_legacy_compatible,
        plan_local_offset_runtime_dispatch_legacy_compatible,
    },
    persistence::{
        load_sorotte_ini_stored_client_settings_mvp_from_path,
        upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    },
    storage::{
        SorotteClientStoragePaths, SorotteClientStorageSource, current_sorotte_client_install_root,
        default_sorotte_client_config_root, ensure_sorotte_client_storage_root, normalize_path,
        persist_sorotte_client_install_locator,
    },
};
use sorotte_player_api::PlayerAdapter;

use super::super::media_match_support::{
    MediaMatchCandidateRebuildRequest, MediaMatchIndexRebuildResult,
    MediaMatchRemoteCandidateRebuildRequest, MediaMatchTool, MediaMatchToolProgress,
    clear_persisted_media_match_cache_at_root, import_managed_media_match_tool_with_progress,
    install_or_update_managed_media_match_tools_with_progress, managed_media_match_bin_dir,
    media_match_cached_probable_candidate_for_remote_signature,
    media_match_inventory_exact_candidate_for_targets, media_match_tool_paths_for_settings,
    rebuild_persisted_media_match_candidates_with_progress_and_cancel,
    rebuild_persisted_media_match_index_with_extraction_settings_and_cancel,
    rebuild_persisted_media_match_remote_candidates_with_progress_and_cancel,
};
use super::super::runtime_bridge::{GuiPendingCompletionRequest, GuiRuntimeRequest};
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiSessionTransportDriver,
    GuiThreadedTcpSessionTransportDriver,
};
use super::super::shell_state::{
    GuiConfigStorageRuntimeSnapshot, GuiConfigurationRuntimeSnapshot, GuiMediaMatchToolHealth,
    GuiMediaSourceProviderId, GuiPluginSelection, GuiShellAction, GuiStreamHelperHealth,
    GuiStreamTargetKind, GuiTransientNotificationLevel, MainWindowRuntimeSnapshot,
    SorotteGuiShellAppState, apply_media_match_settings_to_stored_settings,
    browser_stream_target_kind,
};
use super::super::startup::resolve_sorotte_gui_config_path_legacy_compatible;
use super::super::startup_support::env_trimmed;
use super::super::stream_support::{
    StreamHelperRemediationProgress, import_managed_stream_helper_downloader_with_progress,
    import_managed_stream_helper_js_runtime_with_progress,
    install_or_update_managed_stream_helper_with_progress, managed_stream_helper_bin_dir,
};
use super::super::support::normalized_editable_text;
use super::{
    GuiMediaMatchBackgroundCancelDisposition, GuiPersistedConfigRuntimeOwner,
    GuiUserMediaTargetResolution,
};

impl GuiPersistedConfigRuntimeOwner {
    fn plugin_enablement_config_path_for_request(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<PathBuf> {
        if let Some(config_path) = self.config_path.clone() {
            return Some(config_path);
        }
        let config_path = projected_state
            .config_storage
            .config_path
            .as_deref()
            .map(PathBuf::from)
            .or_else(resolve_sorotte_gui_config_path_legacy_compatible)?;
        self.config_path = Some(config_path.clone());
        Some(config_path)
    }

    fn stop_disabled_plugin_runtime_work(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        plugin: GuiPluginSelection,
    ) {
        match plugin {
            GuiPluginSelection::StreamSupport => {
                self.startup_stream_helper_probe_completed = true;
                self.startup_stream_helper_probe_rx = None;
                self.pending_stream_retry_target = None;
                self.managed_stream_helper_refresh_required = false;
                self.clear_stream_helper_remediation_progress(handle, projected_state);
            }
            GuiPluginSelection::MediaMatching => {
                self.request_media_match_background_worker_cancel(
                    handle,
                    projected_state,
                    GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                    "canceling background work: Media Matching plugin disabled",
                );
                self.last_published_local_file = None;
                self.last_published_media_match_signature = None;
                self.local_shared_playlist_media_match_signature_path = None;
                self.media_match_remote_lookup_rx = None;
                self.media_match_remote_lookup_trigger_key = None;
                self.media_match_remote_lookup_result = None;
                self.media_match_wire_sync_token = None;
                self.clear_pending_playlist_source_resolution_for_provider(
                    &GuiMediaSourceProviderId::media_matching(),
                );
                self.clear_media_match_remediation_progress(handle, projected_state);
                let _ = self.maybe_sync_media_match_wire_decisions(handle, projected_state);
            }
            GuiPluginSelection::Plex => {
                self.plex_auth_session = None;
                self.plex_auth_start_rx = None;
                self.plex_auth_poll_rx = None;
                self.plex_auth_poll_due_at = None;
                self.invalidate_plex_operation_context(handle, projected_state);
                self.clear_pending_playlist_source_resolution_for_provider(
                    &GuiMediaSourceProviderId::plex_stream(),
                );
            }
        }
    }

    pub(in crate::app::runtime_owner) fn push_plugin_disabled_notification(
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        plugin: GuiPluginSelection,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: format!("{} plugin is disabled.", plugin.label()),
            }],
        );
    }

    fn handle_set_plugin_enabled_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        plugin: GuiPluginSelection,
        enabled: bool,
    ) -> bool {
        projected_state
            .plugin_enablement
            .set_enabled_for(plugin, enabled);
        projected_state
            .plugin_enablement
            .apply_to_stored_settings(&mut projected_state.configuration.settings);
        if !enabled {
            self.stop_disabled_plugin_runtime_work(handle, projected_state, plugin);
        }
        let Some(config_path) = self.plugin_enablement_config_path_for_request(projected_state)
        else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not persist {} plugin setting: no writable GUI config path is available.",
                    plugin.label()
                ),
            );
            return false;
        };
        if let Err(error) = upsert_sorotte_ini_stored_client_settings_mvp_at_path(
            &config_path,
            &projected_state.configuration.settings,
        ) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not persist {} plugin setting: {error}",
                    plugin.label()
                ),
            );
            return false;
        }
        let settings = projected_state.configuration.settings.clone();
        let mut actions = vec![
            GuiShellAction::SetPluginEnabled { plugin, enabled },
            GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(GuiConfigurationRuntimeSnapshot {
                draft_settings: settings.clone(),
                saved_settings: settings,
            }),
        ];
        if enabled && plugin == GuiPluginSelection::MediaMatching {
            actions.push(GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
                self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings),
            ));
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        true
    }

    pub(super) fn handle_runtime_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        request: GuiRuntimeRequest,
    ) -> bool {
        match request {
            GuiRuntimeRequest::CheckForUpdates { .. }
            | GuiRuntimeRequest::DownloadUpdate(_)
            | GuiRuntimeRequest::DownloadAndInstallUpdate(_)
            | GuiRuntimeRequest::ApplyStagedUpdate(_) => {
                unreachable!("update requests are routed through GuiClientCommand::Updates")
            }
            GuiRuntimeRequest::UndoSeek
            | GuiRuntimeRequest::SetOffset(_)
            | GuiRuntimeRequest::SetAutoplayEnabled(_)
            | GuiRuntimeRequest::SetAutoplayThreshold(_)
            | GuiRuntimeRequest::RetryPlayerLaunch
            | GuiRuntimeRequest::SeekOffset(_)
            | GuiRuntimeRequest::SeekToPosition(_)
            | GuiRuntimeRequest::KeepWaitingForSeekPreparation
            | GuiRuntimeRequest::CancelSeekPreparation
            | GuiRuntimeRequest::JoinNearestBufferedSeekPreparation
            | GuiRuntimeRequest::SetPlaybackPaused(_)
            | GuiRuntimeRequest::TogglePlaybackPause => {
                unreachable!("player requests are routed through GuiClientCommand::Player")
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
            GuiRuntimeRequest::ImportSharedPlaylistFile { path, shuffled } => {
                self.import_shared_playlist_file_runtime(handle, projected_state, path, shuffled);
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
            GuiRuntimeRequest::SetPluginEnabled { plugin, enabled } => {
                return self.handle_set_plugin_enabled_request(
                    handle,
                    projected_state,
                    plugin,
                    enabled,
                );
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
            GuiRuntimeRequest::InstallMediaMatchTools => {
                return self.handle_install_media_match_tools_request(handle, projected_state);
            }
            GuiRuntimeRequest::ImportMediaMatchFfmpeg(source_path) => {
                return self.handle_import_media_match_tool_request(
                    handle,
                    projected_state,
                    MediaMatchTool::Ffmpeg,
                    source_path,
                );
            }
            GuiRuntimeRequest::ImportMediaMatchFfprobe(source_path) => {
                return self.handle_import_media_match_tool_request(
                    handle,
                    projected_state,
                    MediaMatchTool::Ffprobe,
                    source_path,
                );
            }
            GuiRuntimeRequest::OpenMediaMatchInstallLocation => {
                return self
                    .handle_open_media_match_install_location_request(handle, projected_state);
            }
            GuiRuntimeRequest::RecheckMediaMatchTools => {
                return self.handle_recheck_media_match_tools_request(handle, projected_state);
            }
            GuiRuntimeRequest::RebuildMediaMatchIndex => {
                return self.handle_rebuild_media_match_index_request(handle, projected_state);
            }
            GuiRuntimeRequest::CancelMediaMatchRebuild => {
                return self.handle_cancel_media_match_rebuild_request(handle, projected_state);
            }
            GuiRuntimeRequest::ClearMediaMatchCache => {
                return self.handle_clear_media_match_cache_request(handle, projected_state);
            }
            GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(enabled) => {
                return self.handle_set_media_match_fingerprinting_request(
                    handle,
                    projected_state,
                    enabled,
                );
            }
            GuiRuntimeRequest::SetMediaMatchBackgroundWarmupEnabled(enabled) => {
                return self.handle_set_media_match_background_warmup_request(
                    handle,
                    projected_state,
                    enabled,
                );
            }
            GuiRuntimeRequest::SetMediaMatchWireSharingEnabled(enabled) => {
                return self.handle_set_media_match_wire_sharing_request(
                    handle,
                    projected_state,
                    enabled,
                );
            }
            GuiRuntimeRequest::SetMediaMatchRuntimeToleranceEnabled(enabled) => {
                return self.handle_set_media_match_runtime_tolerance_request(
                    handle,
                    projected_state,
                    enabled,
                );
            }
            GuiRuntimeRequest::SetMediaMatchAutoplayPolicy(policy) => {
                return self.handle_set_media_match_autoplay_policy_request(
                    handle,
                    projected_state,
                    policy,
                );
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
            GuiRuntimeRequest::TogglePlexStreaming(enabled) => {
                return self.handle_toggle_plex_streaming_request(handle, projected_state, enabled);
            }
            GuiRuntimeRequest::DisconnectPlex => {
                return self.handle_disconnect_plex_request(handle, projected_state);
            }
            GuiRuntimeRequest::CancelPlexPlaylistJobs { reason } => {
                self.cancel_plex_playlist_jobs(handle, projected_state, reason);
                return true;
            }
            GuiRuntimeRequest::SearchSelectedPlexServerMedia { query } => {
                return self.handle_search_selected_plex_server_media_request(
                    handle,
                    projected_state,
                    query,
                );
            }
            GuiRuntimeRequest::ResolvePlexPlaylistItem { rating_key } => {
                return self.handle_resolve_plex_playlist_item_request(
                    handle,
                    projected_state,
                    rating_key,
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
                    password.into_exposed_secret(),
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
            GuiRuntimeRequest::ResolvePlaylistSource { index, provider_id } => {
                return self.handle_resolve_playlist_source_request(
                    handle,
                    projected_state,
                    index,
                    provider_id,
                );
            }
            GuiRuntimeRequest::SendChatMessage(message) => {
                return self.handle_send_chat_message_request(handle, projected_state, message);
            }
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SetPlaybackPause(paused),
            ) => {
                return self.handle_complete_set_playback_pause_request(
                    handle,
                    projected_state,
                    paused,
                );
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
                GuiPendingCompletionRequest::DiscardConfigurationChanges(settings),
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
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::ChangeConfigStorageRoot { target, settings },
            ) => {
                return self.handle_complete_config_storage_root_change_request(
                    handle,
                    projected_state,
                    target,
                    settings,
                );
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
