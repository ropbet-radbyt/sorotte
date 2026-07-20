use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_misc_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::RetryPlayerLaunch
            | GuiShellAction::RetryPlayerSettings
            | GuiShellAction::RetryChatOsdIntegration => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetPluginEnabled { plugin, enabled } => {
                self.plugin_enablement.set_enabled_for(plugin, enabled);
                self.plugin_enablement
                    .apply_to_stored_settings(&mut self.configuration.settings);
                self.refresh_playlist_source_states();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::InstallStreamHelper
            | GuiShellAction::IntegrateStreamHelperDownloader(_)
            | GuiShellAction::IntegrateStreamHelperJsRuntime(_)
            | GuiShellAction::RecheckStreamHelper
            | GuiShellAction::OpenStreamHelperInstallLocation
            | GuiShellAction::RetryPendingStreamMediaOpen
            | GuiShellAction::InstallMediaMatchTools
            | GuiShellAction::ImportMediaMatchFfmpeg(_)
            | GuiShellAction::ImportMediaMatchFfprobe(_)
            | GuiShellAction::RecheckMediaMatchTools
            | GuiShellAction::RebuildMediaMatchIndex
            | GuiShellAction::CancelMediaMatchRebuild
            | GuiShellAction::ClearMediaMatchCache
            | GuiShellAction::OpenMediaMatchInstallLocation
            | GuiShellAction::StartPlexAuth
            | GuiShellAction::PollPlexAuth
            | GuiShellAction::RefreshPlexServers
            | GuiShellAction::SelectPlexServer { .. }
            | GuiShellAction::TogglePlexSync(_)
            | GuiShellAction::TogglePlexStreaming(_)
            | GuiShellAction::DisconnectPlex => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetMediaMatchFingerprintingEnabled(enabled) => {
                self.media_match.settings.fingerprinting_enabled = enabled;
                apply_media_match_settings_to_stored_settings(
                    &mut self.configuration.settings,
                    &self.media_match.settings,
                );
                self.refresh_playlist_source_states();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(enabled) => {
                self.media_match.settings.background_warmup_enabled = enabled;
                apply_media_match_settings_to_stored_settings(
                    &mut self.configuration.settings,
                    &self.media_match.settings,
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetMediaMatchWireSharingEnabled(enabled) => {
                self.media_match.settings.wire_sharing_enabled = enabled;
                apply_media_match_settings_to_stored_settings(
                    &mut self.configuration.settings,
                    &self.media_match.settings,
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(enabled) => {
                self.media_match.settings.runtime_tolerance_enabled = enabled;
                apply_media_match_settings_to_stored_settings(
                    &mut self.configuration.settings,
                    &self.media_match.settings,
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SetMediaMatchAutoplayPolicy(policy) => {
                self.media_match.settings.autoplay_policy = policy;
                apply_media_match_settings_to_stored_settings(
                    &mut self.configuration.settings,
                    &self.media_match.settings,
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ToggleMainWindowPlaybackButtons => {
                self.toggle_main_window_playback_buttons()
            }
            GuiShellAction::ToggleMainWindowAutoplayControls => {
                self.toggle_main_window_autoplay_controls()
            }
            GuiShellAction::ToggleMainWindowHideEmptyRooms => {
                self.toggle_main_window_hide_empty_rooms()
            }
            GuiShellAction::ToggleMainWindowRoomChange => {
                self.main_window_room_change_expanded = !self.main_window_room_change_expanded;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RequestMainWindowUserMediaOpen(_) => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RequestMainWindowUserContainingFolderOpen(_) => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RequestMainWindowUserReady { .. }
            | GuiShellAction::RequestControllerAuth { .. } => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AddTrustedDomain(domain) => self.add_trusted_domain(domain),
            GuiShellAction::JoinMainWindowRoom(room) => self.join_main_window_room(room),
            GuiShellAction::LeaveMainWindowRoom => self.leave_main_window_room(),
            GuiShellAction::SetMainWindowRoom(room) => {
                let Some(room) = nonempty_room_name_text(&room) else {
                    return self.record_action_error("Room name cannot be empty.");
                };
                self.set_main_window_room_state(Some(room));
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                self.apply_main_window_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiRuntimeSnapshot(snapshot) => {
                self.apply_gui_runtime_snapshot(snapshot)
            }
            GuiShellAction::PushChatMessage { sender, message } => {
                if sender.trim().is_empty() || message.trim().is_empty() {
                    return self
                        .record_action_error("Chat sender and message must both be non-empty.");
                }
                self.append_chat_row(sender, message);
                self.clear_action_error_and_refresh();
                true
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
