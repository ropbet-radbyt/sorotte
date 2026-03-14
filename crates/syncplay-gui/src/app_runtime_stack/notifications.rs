use syncplay_client_core::{
    AutoplayCountdownNotification, ControlledRoomCreationNotification,
    ControllerAuthTransitionNotification, ReconnectTransitionNotification, UserChangeNotification,
};

use super::super::shell_state::{GuiShellAction, GuiTransientNotificationLevel};
use super::GuiClientCoreChatSessionRuntimeAdapter;

impl GuiClientCoreChatSessionRuntimeAdapter {
    pub(super) fn note_user_change(&mut self, notification: UserChangeNotification) {
        match notification {
            UserChangeNotification::Joined { username, .. }
            | UserChangeNotification::Playing { username, .. } => {
                self.tracked_remote_usernames.insert(username);
            }
            UserChangeNotification::Left { username, .. } => {
                self.tracked_remote_usernames.remove(&username);
            }
        }
    }

    pub(super) fn user_change_action(
        notification: UserChangeNotification,
    ) -> Option<GuiShellAction> {
        let message = match notification {
            UserChangeNotification::Joined {
                username,
                room,
                hide_from_osd,
            } => (!hide_from_osd).then(|| format!("{username} joined {room}.")),
            UserChangeNotification::Playing {
                username,
                room,
                file_name,
                include_room_addendum,
                hide_from_osd,
                ..
            } => {
                if hide_from_osd {
                    None
                } else {
                    let media_label = file_name.unwrap_or_else(|| "media".to_owned());
                    let room_addendum = if include_room_addendum {
                        format!(" in {room}")
                    } else {
                        String::new()
                    };
                    Some(format!(
                        "{username} is playing {media_label}{room_addendum}."
                    ))
                }
            }
            UserChangeNotification::Left {
                username,
                hide_from_osd,
            } => (!hide_from_osd).then(|| format!("{username} left.")),
        }?;
        Some(GuiShellAction::AnnounceSystemChatEvent(message))
    }

    pub(in crate::app) fn reconnect_transition_actions(
        notification: ReconnectTransitionNotification,
    ) -> Vec<GuiShellAction> {
        let (level, message, persist_to_system_chat) = match notification {
            ReconnectTransitionNotification::Attempting {
                retries,
                delay_seconds,
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Reconnect attempt {} in {:.1} seconds.",
                    retries.saturating_add(1),
                    delay_seconds
                ),
                true,
            ),
            ReconnectTransitionNotification::Connected => (
                GuiTransientNotificationLevel::Success,
                "Session reconnected.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::Disconnected => (
                GuiTransientNotificationLevel::Warning,
                "Session disconnected.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::RestoringState => (
                GuiTransientNotificationLevel::Info,
                "Restoring session state.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                position_diff_seconds,
                ..
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Session state restore mismatch detected ({position_diff_seconds:.3} seconds)."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt,
                max_attempts,
                cooldown_ticks,
            } => (
                GuiTransientNotificationLevel::Warning,
                format!(
                    "Session state correction retry {attempt}/{max_attempts} scheduled after {cooldown_ticks} ticks."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts,
                max_attempts,
            } => (
                GuiTransientNotificationLevel::Error,
                format!(
                    "Session state correction exhausted after {attempts}/{max_attempts} attempts."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles,
                disable_after_mismatch_cycles,
            } => (
                GuiTransientNotificationLevel::Error,
                format!(
                    "Session state correction disabled after {consecutive_mismatch_cycles}/{disable_after_mismatch_cycles} mismatch cycles."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                remaining_reconnect_cycles_after_this_cycle,
            } => (
                GuiTransientNotificationLevel::Info,
                format!(
                    "Session state correction recovery cooldown active for {remaining_reconnect_cycles_after_this_cycle} more reconnect cycles."
                ),
                true,
            ),
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled => (
                GuiTransientNotificationLevel::Info,
                "Session state correction recovery cooldown ended.".to_owned(),
                true,
            ),
            ReconnectTransitionNotification::RestoringPlaylist => (
                GuiTransientNotificationLevel::Info,
                "Restoring shared playlist state.".to_owned(),
                true,
            ),
        };
        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level,
            message: message.clone(),
        }];
        if persist_to_system_chat {
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        actions
    }

    pub(super) fn controller_auth_transition_action(
        notification: ControllerAuthTransitionNotification,
    ) -> Vec<GuiShellAction> {
        let (level, message) = match notification {
            ControllerAuthTransitionNotification::Attempting { room } => (
                GuiTransientNotificationLevel::Info,
                format!("Requesting controller access for {room}."),
            ),
            ControllerAuthTransitionNotification::Succeeded {
                username,
                room,
                hide_from_osd,
            } => {
                if hide_from_osd {
                    return Vec::new();
                }
                (
                    GuiTransientNotificationLevel::Success,
                    format!("{username} received controller access for {room}."),
                )
            }
            ControllerAuthTransitionNotification::Failed {
                username,
                room,
                hide_from_osd,
            } => {
                if hide_from_osd {
                    return Vec::new();
                }
                (
                    GuiTransientNotificationLevel::Error,
                    format!("Controller access failed for {username} in {room}."),
                )
            }
        };
        vec![
            GuiShellAction::PushTransientNotification {
                level,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }

    pub(super) fn controlled_room_creation_action(
        notification: ControlledRoomCreationNotification,
    ) -> Vec<GuiShellAction> {
        match notification {
            ControlledRoomCreationNotification::Created { room, password } => {
                let share_code = format!("{room}:{password}");
                let transient_message = format!("Controlled room created: {room}.");
                let chat_message = format!(
                    "Created controlled room {room} with password {password} ({share_code})."
                );
                vec![
                    GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Success,
                        message: transient_message,
                    },
                    GuiShellAction::AnnounceSystemChatEvent(chat_message),
                ]
            }
        }
    }

    pub(super) fn autoplay_countdown_action(
        notification: AutoplayCountdownNotification,
    ) -> Vec<GuiShellAction> {
        let message = format!(
            "Autoplay in {} seconds with {} ready users.",
            notification.seconds_left, notification.ready_user_count
        );
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]
    }
}
