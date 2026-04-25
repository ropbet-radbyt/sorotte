use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn seek_unavailable_message_impl(
        &self,
        offset_seconds: f64,
    ) -> String {
        let base = format!(
            "Playback seek requires a playback runtime connection; the {offset_seconds} second request was not applied."
        );
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(in crate::app::runtime_owner) fn toggle_pause_unavailable_message_impl(&self) -> String {
        let base =
            "Playback toggle requires a playback runtime connection; the pause request was not applied."
                .to_owned();
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(in crate::app::runtime_owner) fn send_chat_unavailable_message_impl(&self) -> String {
        "Chat input is unavailable because no session runtime is connected. The message was not sent."
            .to_owned()
    }

    pub(in crate::app::runtime_owner) fn push_player_success_impl(
        handle: &GuiQueuedRuntimeBridgeHandle,
        message: String,
    ) {
        handle.push_actions([
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(in crate::app::runtime_owner) fn push_player_error_impl(
        handle: &GuiQueuedRuntimeBridgeHandle,
        message: String,
    ) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }
}
