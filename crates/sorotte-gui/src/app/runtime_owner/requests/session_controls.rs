use super::*;
use crate::app::runtime_owner::GuiPendingSharedPlaylistOpen;
use crate::app::runtime_stack::GuiPlaylistProtocolDeliveryFence;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn arm_playlist_player_effect_delivery_fence(
        &mut self,
        delivery_fence: GuiPlaylistProtocolDeliveryFence,
    ) {
        if delivery_fence.is_reached() {
            return;
        }
        if let Some(pending) = self.pending_shared_playlist_open.as_mut() {
            pending.replace_delivery_fence(delivery_fence);
        } else {
            self.pending_shared_playlist_open =
                Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { delivery_fence });
        }
    }

    pub(super) fn handle_set_local_ready_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        ready: bool,
    ) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.set_local_ready(ready)
        {
            handle.push_action(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error,
            });
        }
        true
    }

    pub(super) fn handle_set_ready_for_user_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        username: String,
        ready: bool,
    ) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.set_user_ready(username, ready)
        {
            handle.push_action(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error,
            });
        }
        true
    }

    pub(super) fn handle_request_controller_auth_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        room: String,
        password: String,
    ) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.request_controller_auth(room, password)
        {
            handle.push_action(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error,
            });
        }
        true
    }

    pub(super) fn handle_queue_playlist_entry_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        entry: String,
        select_after_queue: bool,
    ) -> bool {
        let selected_media_can_change = select_after_queue;
        if let Some(session) = self.session.as_mut() {
            match session.queue_playlist_entry_with_delivery_fence(entry, select_after_queue) {
                Ok(delivery_fence) if selected_media_can_change => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Ok(_) => {}
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_set_playlist_index_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.set_playlist_index_with_delivery_fence(index) {
                Ok(delivery_fence) => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence);
                    self.active_shared_playlist_index = Some(index);
                    projected_state.main_window.active_playlist_index = Some(index);
                    handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                        MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                    ));
                }
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        } else if projected_state.main_window.playlist.get(index).is_some() {
            self.active_shared_playlist_index = Some(index);
            projected_state.main_window.active_playlist_index = Some(index);
            handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
            ));
            let selected_media_sync =
                self.sync_selected_shared_playlist_media_to_attached_player_impl(projected_state);
            let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
                self.session
                    .as_ref()
                    .is_some_and(|session| session.has_pending_playlist_index_reset_intent()),
            );
            self.apply_pending_playlist_index_reset_to_attached_player_impl(
                projected_state,
                selection_handoff_ready,
            );
            self.sync_session_playstate_to_attached_player_impl(
                projected_state,
                selection_handoff_ready,
            );
        }
        true
    }

    pub(super) fn handle_delete_playlist_index_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        index: usize,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.delete_playlist_index_with_delivery_fence(index) {
                Ok(delivery_fence) => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_undo_playlist_change_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.undo_playlist_change_with_delivery_fence() {
                Ok(delivery_fence) => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_shuffle_remaining_playlist_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.shuffle_remaining_playlist_with_delivery_fence() {
                Ok(delivery_fence) => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_shuffle_entire_playlist_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.shuffle_entire_playlist_with_delivery_fence() {
                Ok(delivery_fence) => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_advance_playlist_index_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if self.session.is_some() {
            if let Err(error) = self.advance_playlist_index_for_attached_player_impl() {
                handle.push_action(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: error,
                });
            }
        } else {
            Self::push_runtime_unavailable(
                handle,
                "Advancing the shared playlist requires an active session runtime.".to_owned(),
            );
        }
        true
    }

    pub(super) fn handle_replace_playlist_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        _projected_state: &mut SorotteGuiShellAppState,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> bool {
        let selected_media_can_change = selected_index.is_some();
        if let Some(session) = self.session.as_mut() {
            match session.replace_playlist_with_delivery_fence(files, selected_index) {
                Ok(delivery_fence) if selected_media_can_change => {
                    self.arm_playlist_player_effect_delivery_fence(delivery_fence)
                }
                Ok(_) => {}
                Err(error) => {
                    handle.push_action(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Error,
                        message: error,
                    });
                }
            }
        }
        true
    }

    pub(super) fn handle_send_chat_message_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        message: String,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            match session.send_chat_message(message) {
                Ok(()) => Self::push_actions_and_project(handle, projected_state, Vec::new()),
                Err(error) => Self::push_runtime_unavailable(
                    handle,
                    format!("Chat sending through the attached session runtime failed: {error}"),
                ),
            }
        } else {
            Self::push_runtime_unavailable(handle, self.send_chat_unavailable_message());
        }
        true
    }
}
