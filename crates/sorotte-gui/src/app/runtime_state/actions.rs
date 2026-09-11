use super::GuiRuntimeState;
use crate::app::shell_state::*;
use crate::app::support::normalized_editable_text;
use crate::app::{configuration_model, feature_snapshots, playlist_model};
use sorotte_client_app::app_boundary::state::StoredClientSettings;

impl GuiRuntimeState {
    /// Apply the feature effects of an output before processing the next
    /// command. Navigation, editors and notification presentation remain UI-owned.
    pub(in crate::app) fn apply(&mut self, action: GuiShellAction) -> bool {
        let result = match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                if !self.apply_main_window_runtime_snapshot(snapshot) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::ApplySharedPlaylistSelection(index) => {
                if self.playlist.selection_is_local {
                    return false;
                }
                if index.is_some_and(|index| index >= self.playlist.main_window.playlist.len()) {
                    return self.record_action_error(
                        "The shared playlist selection refers to a missing row.",
                    );
                }
                self.playlist.selection.selected_main_window_playlist = index;
                self.apply_selection_to_surfaces();
                Ok(())
            }
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot) => {
                if let Err(message) = feature_snapshots::apply_menu_dialog_snapshot(
                    &mut self.session.menus,
                    &mut self.session.menu_overrides,
                    &self.settings.draft.to_stored_settings(),
                    snapshot,
                ) {
                    return self.record_action_error(message);
                }
                self.sync_dialog_menu_actions_from_runtime_state();
                self.normalize_selected_menu_action_after_runtime_update();
                self.apply_selection_to_surfaces();
                Ok(())
            }
            GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_media_index_runtime_snapshot(
                    &mut self.media_resolution.index_status,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_player_setup_runtime_snapshot(
                    &mut self.player.setup_issue,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_seek_preparation_runtime_snapshot(
                    &mut self.player.seek_preparation,
                    &mut self.player.seek_preparation_degraded_reason,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_stream_helper_runtime_snapshot(
                    &mut self.player.stream_helper,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_stream_helper_remediation_runtime_snapshot(
                    &mut self.player.stream_helper_remediation,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot) => {
                let result = feature_snapshots::apply_gui_media_match_runtime_snapshot(
                    &mut self.media_match.model,
                    snapshot,
                );
                if result.is_ok() {
                    self.refresh_playlist_source_states();
                }
                result
            }
            GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(snapshot) => {
                feature_snapshots::apply_gui_media_match_remediation_runtime_snapshot(
                    &mut self.media_match.remediation,
                    snapshot,
                )
            }
            GuiShellAction::ApplyGuiPlexRuntimeSnapshot(snapshot) => {
                let result = feature_snapshots::apply_gui_plex_runtime_snapshot(
                    &mut self.plex.model,
                    snapshot,
                );
                if result.is_ok() {
                    self.refresh_playlist_source_states();
                }
                result
            }
            GuiShellAction::ApplyGuiPersistedSettingsPatch(patch) => {
                if configuration_model::apply_persisted_settings_patch(
                    &mut self.settings.saved,
                    &mut self.settings.draft,
                    &mut self.settings.plugin_enablement,
                    &mut self.media_match.model,
                    &mut self.plex.model,
                    patch,
                ) {
                    self.refresh_playlist_source_states();
                }
                Ok(())
            }
            GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(snapshot) => {
                self.settings.config_storage = snapshot;
                Ok(())
            }
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(snapshot) => {
                if snapshot.pending_operation.is_some()
                    && snapshot.command_availability.any_enabled()
                {
                    return self.record_action_error("GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active.");
                }
                if snapshot.pending_operation
                    != self
                        .session
                        .pending_operation
                        .as_ref()
                        .map(|pending| pending.kind)
                {
                    return false;
                }
                self.session.command_overrides =
                    GuiCommandAvailabilityRuntimeOverride::from_baseline_and_snapshot(
                        &self.command_availability_without_runtime_override(),
                        &snapshot.command_availability,
                    );
                Ok(())
            }
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(snapshot) => {
                if self.configuration_operation_pending() {
                    return self.record_action_error("GUI saved-configuration runtime snapshots cannot apply while a configuration command is already in progress.");
                }
                if self.session.pending_saved_server_connect_intent
                    == Some(GuiSavedServerConnectIntent::SaveAndConnect)
                    && self.pending_operation_is(GuiPendingOperationKind::ConnectSavedServer)
                {
                    self.settle_persisted_configuration(snapshot.settings);
                } else {
                    self.settings.saved = snapshot.settings;
                }
                Ok(())
            }
            GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(snapshot) => {
                if self.configuration_operation_pending() {
                    return false;
                }
                self.resync_from_settings(snapshot.draft_settings);
                self.settings.saved = snapshot.saved_settings;
                Ok(())
            }
            GuiShellAction::CompleteConfigurationSave(settings) => {
                if !self.finish_configuration_operation(
                    GuiPendingOperationKind::SaveConfiguration,
                    settings,
                ) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CompleteDiscardConfigurationChanges(settings) => {
                if !self.finish_configuration_operation(
                    GuiPendingOperationKind::DiscardConfigurationChanges,
                    settings,
                ) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CompleteConfigurationReload(settings) => {
                if !self.finish_configuration_operation(
                    GuiPendingOperationKind::ReloadConfiguration,
                    settings,
                ) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings } => {
                if !self.pending_operation_is(GuiPendingOperationKind::ChangeConfigStorageRoot) {
                    return false;
                }
                self.settle_persisted_configuration(settings);
                self.settings.config_storage = snapshot;
                self.settings.pending_storage_target = None;
                self.session.pending_operation = None;
                self.session.pending_saved_server_connect_intent = None;
                Ok(())
            }
            GuiShellAction::CompletePendingOperation => {
                let Some(pending) = self.session.pending_operation.take() else {
                    return false;
                };
                if pending.kind == GuiPendingOperationKind::SendChatMessage {
                    self.session.outgoing_chat_message = None;
                }
                self.session.pending_saved_server_connect_intent = None;
                Ok(())
            }
            GuiShellAction::CompleteSavedServerConnect => {
                if !self.pending_operation_is(GuiPendingOperationKind::ConnectSavedServer) {
                    return self.record_action_error(
                        "No configured-server connect is currently in progress.",
                    );
                }
                let Some(intent) = self.session.pending_saved_server_connect_intent else {
                    self.session.pending_operation = None;
                    return self.record_action_error(
                        "Configured server connect is missing its submitted connection intent.",
                    );
                };
                let settings = self.submitted_saved_server_connect_settings(intent);
                let runtime_settings = sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot(&settings);
                self.session.pending_operation = None;
                self.session.pending_saved_server_connect_intent = None;
                if crate::app::runtime_owner::GuiPersistedConfigRuntimeOwner::saved_server_connect_target_for_runtime_settings(&runtime_settings).is_none() {
                    return self.record_action_error("Configured server connect requires a saved host and a valid port.");
                }
                Ok(())
            }
            GuiShellAction::CompleteSelectedPublicServerConnect => {
                if !self.pending_operation_is(GuiPendingOperationKind::ConnectPublicServer) {
                    return self
                        .record_action_error("No public server connect is currently in progress.");
                }
                self.session.pending_operation = None;
                if !self
                    .session
                    .public_servers
                    .servers
                    .iter()
                    .any(|row| row.is_selected)
                {
                    return self.record_action_error("No public server is currently selected.");
                }
                Ok(())
            }
            GuiShellAction::CompleteSessionDisconnect => {
                if !self.pending_operation_is(GuiPendingOperationKind::DisconnectSession) {
                    return false;
                }
                self.session.pending_operation = None;
                Ok(())
            }
            GuiShellAction::CompleteLocalChatSend => {
                if self.session.pending_operation.is_some() {
                    if !self.pending_operation_is(GuiPendingOperationKind::SendChatMessage) {
                        return false;
                    }
                    self.session.pending_operation = None;
                    self.session.outgoing_chat_message = None;
                }
                Ok(())
            }
            GuiShellAction::CompletePlaybackPauseToggle => {
                if !self.pending_operation_is(GuiPendingOperationKind::TogglePlaybackPause) {
                    return false;
                }
                self.session.pending_operation = None;
                if !self.set_playback_paused(!self.playlist.main_window.playback_paused, false) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CompletePlaybackPauseState(paused) => {
                if !self
                    .session
                    .pending_operation
                    .as_ref()
                    .is_some_and(|pending| {
                        matches!(
                            pending.kind,
                            GuiPendingOperationKind::SetPlaybackPause(_)
                                | GuiPendingOperationKind::TogglePlaybackPause
                        )
                    })
                {
                    return false;
                }
                self.session.pending_operation = None;
                if self.playlist.main_window.playback_paused != paused
                    && !self.set_playback_paused(paused, false)
                {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::AnnouncePlaybackPaused => {
                if !self.set_playback_paused(true, true) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::AnnouncePlaybackResumed => {
                if !self.set_playback_paused(false, true) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::AnnounceSharedPlaylistLoaded(entries) => {
                if !self.playlist.main_window.shared_playlist_enabled {
                    return self.record_action_error("Shared playlist events are unavailable when shared playlists are disabled.");
                }
                let entries = playlist_model::normalize_shared_playlist_entries(entries);
                self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
                let selection = (!entries.is_empty()).then_some(0);
                self.apply_shared_playlist_entries(entries, selection, false);
                self.push_system_chat_message(if selection.is_none() {
                    "Shared playlist cleared.".to_owned()
                } else {
                    format!(
                        "Shared playlist loaded ({} entries).",
                        self.playlist.main_window.playlist.len()
                    )
                });
                Ok(())
            }
            GuiShellAction::AppendSharedPlaylistEntries(entries) => {
                if !self.playlist.main_window.shared_playlist_enabled {
                    return false;
                }
                let current = self.current_shared_playlist_entries();
                let additions = playlist_model::unique_shared_playlist_additions(&current, entries);
                if !additions.is_empty() {
                    let mut next = current.clone();
                    next.extend(additions.iter().cloned());
                    let index = playlist_model::shared_playlist_target_index_from_changed_entries(
                        &current,
                        self.playlist.selection.selected_main_window_playlist,
                        &next,
                    )
                    .min(next.len().saturating_sub(1));
                    self.remember_shared_playlist_undo_snapshot_if_changed(&next);
                    self.apply_shared_playlist_entries(next, Some(index), true);
                    self.push_system_chat_message(if additions.len() == 1 {
                        format!("Shared playlist entry added: {}.", additions[0])
                    } else {
                        format!("Shared playlist entries added: {} items.", additions.len())
                    });
                }
                Ok(())
            }
            GuiShellAction::CompletePlexPlaylistSearch {
                query,
                results,
                error,
            } => {
                if !feature_snapshots::complete_plex_playlist_search(
                    &mut self.plex.playlist_search,
                    query,
                    results,
                    error,
                ) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CompletePlexPlaylistItemResolve { rating_key, error } => {
                if !feature_snapshots::complete_plex_playlist_item_resolve(
                    &mut self.plex.playlist_search,
                    rating_key,
                    error,
                ) {
                    return false;
                }
                Ok(())
            }
            GuiShellAction::CancelPlexPlaylistSearch => {
                self.plex.playlist_search = None;
                Ok(())
            }
            GuiShellAction::PushChatMessage { sender, message } => {
                if sender.trim().is_empty() || message.trim().is_empty() {
                    return self
                        .record_action_error("Chat sender and message must both be non-empty.");
                }
                self.playlist
                    .main_window
                    .chat
                    .push(MainWindowChatRow { sender, message });
                Ok(())
            }
            GuiShellAction::AnnounceSystemChatEvent(message) => {
                let Some(message) = normalized_editable_text(&message) else {
                    return self.record_action_error("System chat messages must be non-empty.");
                };
                self.push_system_chat_message(message);
                Ok(())
            }
            GuiShellAction::AnnounceControlledRoomCreated { room, password } => {
                let password = password.expose_secret();
                self.push_system_chat_message(format!(
                    "Created controlled room {room} with password {password} ({room}:{password})."
                ));
                Ok(())
            }
            action => return self.apply_service_output(action),
        };
        match result {
            Ok(()) => {
                self.refresh_update_policy();
                self.clear_action_error_and_refresh();
                true
            }
            Err(message) => self.record_action_error(message),
        }
    }

    pub(super) fn pending_operation_is(&self, kind: GuiPendingOperationKind) -> bool {
        self.session
            .pending_operation
            .as_ref()
            .is_some_and(|pending| pending.kind == kind)
    }

    fn configuration_operation_pending(&self) -> bool {
        self.session
            .pending_operation
            .as_ref()
            .is_some_and(|pending| {
                matches!(
                    pending.kind,
                    GuiPendingOperationKind::SaveConfiguration
                        | GuiPendingOperationKind::DiscardConfigurationChanges
                        | GuiPendingOperationKind::ReloadConfiguration
                )
            })
    }

    fn finish_configuration_operation(
        &mut self,
        kind: GuiPendingOperationKind,
        settings: StoredClientSettings,
    ) -> bool {
        if !self.pending_operation_is(kind) {
            return false;
        }
        self.settle_persisted_configuration(settings);
        self.session.pending_operation = None;
        self.settings.pending_storage_target = None;
        self.session.pending_saved_server_connect_intent = None;
        true
    }
}

impl GuiRuntimeState {
    pub(super) fn push_system_chat_message(&mut self, message: String) {
        let message = crate::app::runtime_localization::localize_gui_runtime_message(
            &message,
            Some(self.runtime_language_tag()),
        );
        self.playlist.main_window.chat.push(MainWindowChatRow {
            sender: "system".to_owned(),
            message,
        });
    }
    fn set_playback_paused(&mut self, paused: bool, announce: bool) -> bool {
        if !self.playlist.main_window.playback.can_toggle_pause {
            return self.record_action_error(
                "Playback pause state cannot change when pause controls are unavailable.",
            );
        }
        if self.playlist.main_window.playback_paused == paused {
            return self.record_action_error(if paused {
                "Playback is already paused."
            } else {
                "Playback is already running."
            });
        }
        self.playlist.main_window.playback_paused = paused;
        if announce {
            self.push_system_chat_message(
                if paused {
                    "Playback paused."
                } else {
                    "Playback resumed."
                }
                .to_owned(),
            );
        }
        true
    }
}
