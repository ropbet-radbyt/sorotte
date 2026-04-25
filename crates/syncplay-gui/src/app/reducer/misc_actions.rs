use super::*;

impl SyncplayGuiShellAppState {
    pub(super) fn apply_misc_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::RetryPlayerLaunch => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::InstallStreamHelper
            | GuiShellAction::IntegrateStreamHelperDownloader(_)
            | GuiShellAction::IntegrateStreamHelperJsRuntime(_)
            | GuiShellAction::RecheckStreamHelper
            | GuiShellAction::OpenStreamHelperInstallLocation
            | GuiShellAction::RetryPendingStreamMediaOpen => {
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
