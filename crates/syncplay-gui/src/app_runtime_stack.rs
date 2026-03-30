#[cfg(test)]
#[path = "app_runtime_stack/tests.rs"]
mod tests;

#[path = "app_runtime_stack/media_search.rs"]
mod media_search;
#[path = "app_runtime_stack/notifications.rs"]
mod notifications;
#[path = "app_runtime_stack_player.rs"]
mod player;
#[path = "app_runtime_stack/public_servers.rs"]
mod public_servers;
#[path = "app_runtime_stack/runtime_snapshots.rs"]
mod runtime_snapshots;
#[path = "app_runtime_stack_transport.rs"]
mod transport;

use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
    time::{Duration, Instant},
};

use serde_json::Value;
use syncplay_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, ChatNotification, ClientRuntime, ClientSession, PrivacyMode,
    QueuedRuntimeControl, RoomPlaylistView,
};
use syncplay_player_api::PlayerPlaybackTelemetryUpdate;
use syncplay_protocol::{ProtocolMessage, decode_message_line};

use self::player::GuiNoopClientRuntimePlayer;
#[cfg(not(test))]
use super::remote_services;
use super::shell_state::{
    GuiCommandAvailabilityState, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SyncplayGuiShellAppState,
};
use super::support::system_time_seconds;

pub(super) use self::player::{GuiOwnedPlayer, GuiPlayerLaunchRuntimeState, GuiTestPlayerAdapter};
pub(super) use self::transport::{
    GuiLoopbackSessionTransportDriver, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
    GuiTcpSessionTransportDriver,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct GuiSessionRoomPlaystate {
    pub(super) position_seconds: Option<f64>,
    pub(super) paused: Option<bool>,
    pub(super) do_seek: Option<bool>,
    pub(super) set_by: Option<String>,
}

pub(super) trait GuiSessionRuntimeAdapter: Send {
    fn drain_gui_actions(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn playlist_control_available(&self) -> bool {
        false
    }

    fn adjust_command_availability(
        &self,
        _state: &SyncplayGuiShellAppState,
        command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        command_availability
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn apply_message_json(&mut self, _json_line: &str) -> Result<(), String> {
        Err(
            "Attached session runtime does not accept inbound protocol transport messages."
                .to_owned(),
        )
    }

    fn set_room(&mut self, _room: String) -> Result<(), String> {
        Err("Attached session runtime does not support room changes.".to_owned())
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        self.set_room(default_room)
    }

    fn set_local_ready(&mut self, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support local readiness changes.".to_owned())
    }

    fn mark_local_media_opened_not_ready(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    fn set_user_ready(&mut self, _username: String, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support remote readiness changes.".to_owned())
    }

    fn request_controller_auth(&mut self, _room: String, _password: String) -> Result<(), String> {
        Err("Attached session runtime does not support controller auth requests.".to_owned())
    }

    fn queue_playlist_entry(
        &mut self,
        _entry: String,
        _select_after_queue: bool,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist queue operations."
                .to_owned(),
        )
    }

    fn set_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist selection changes."
                .to_owned(),
        )
    }

    fn advance_playlist_index(&mut self) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist advancement.".to_owned())
    }

    fn delete_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist removal.".to_owned())
    }

    fn replace_playlist(
        &mut self,
        _files: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist reorder operations."
                .to_owned(),
        )
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist undo.".to_owned())
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn sync_local_playback_telemetry(
        &mut self,
        _paused: Option<bool>,
        _position_seconds: Option<f64>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn set_playback_paused(&mut self, _paused: bool) -> Result<bool, String> {
        Err("Attached session runtime does not support playback pause changes.".to_owned())
    }

    fn record_manual_seek_to_position(&mut self, _position_seconds: f64) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek history.".to_owned())
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek undo.".to_owned())
    }

    fn local_position_seconds(&self) -> Option<f64> {
        None
    }

    fn local_username(&self) -> Option<&str> {
        None
    }

    fn current_room_playstate(&self) -> Option<GuiSessionRoomPlaystate> {
        None
    }

    fn current_room_playstate_for_attached_player_sync(&self) -> Option<GuiSessionRoomPlaystate> {
        self.current_room_playstate()
    }

    fn note_local_playlist_index_reset_intent(&mut self, _pause_before_sync: bool) {}

    fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        None
    }

    fn has_pending_playlist_index_reset_intent(&self) -> bool {
        false
    }

    fn set_autoplay_enabled(&mut self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, _threshold: usize) -> Result<(), String> {
        Ok(())
    }

    fn publish_local_file_legacy_compatible(
        &mut self,
        _file_payload: &Value,
        _filename_privacy_mode: PrivacyMode,
        _filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), String> {
        Ok(())
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String>;

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String>;

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
        language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String>;

    fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        Err("Attached session runtime does not expose a missing-media search target.".to_owned())
    }

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String>;
}

#[allow(dead_code)]
pub(super) struct GuiClientCoreChatSessionRuntimeAdapter {
    username: String,
    baseline_room: String,
    pub(super) runtime: ClientRuntime<GuiNoopClientRuntimePlayer, QueuedRuntimeControl>,
    pending_startup_protocol_lines: VecDeque<String>,
    next_state_sync_heartbeat_at: Option<Instant>,
    next_autoplay_tick_at: Option<Instant>,
    tracked_remote_usernames: BTreeSet<String>,
    optimistic_room_playlist: Option<(String, RoomPlaylistView)>,
}

#[allow(dead_code)]
impl GuiClientCoreChatSessionRuntimeAdapter {
    const STATE_SYNC_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

    pub(super) fn new(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        Self::new_with_control_password(username, room, None)
    }

    pub(super) fn new_with_control_password(
        username: impl Into<String>,
        room: impl Into<String>,
        controlled_room_password_override: Option<String>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let hello_json = Self::hello_json(&username, &room);
        let mut session = ClientSession::default();
        if let Some(control_password) = controlled_room_password_override.as_deref() {
            session.remember_control_password_for_room(&room, control_password);
        }

        Ok(Self {
            username,
            baseline_room: room,
            runtime: ClientRuntime::new(
                session,
                GuiNoopClientRuntimePlayer,
                QueuedRuntimeControl::default(),
            ),
            pending_startup_protocol_lines: VecDeque::from([hello_json]),
            next_state_sync_heartbeat_at: None,
            next_autoplay_tick_at: None,
            tracked_remote_usernames: BTreeSet::new(),
            optimistic_room_playlist: None,
        })
    }

    fn hello_json(username: &str, room: &str) -> String {
        format!(
            r#"{{"Hello":{{"username":{username:?},"room":{{"name":{room:?}}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
        )
    }

    fn current_room_for_next_hello(&self) -> String {
        self.runtime
            .session()
            .local_room_command_target_with_legacy_fallback(&self.baseline_room)
    }

    fn reset_session_for_reconnect(&mut self) {
        let room = self.current_room_for_next_hello();
        self.baseline_room = room.clone();
        self.runtime = ClientRuntime::new(
            ClientSession::default(),
            GuiNoopClientRuntimePlayer,
            QueuedRuntimeControl::default(),
        );
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(&self.username, &room));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.tracked_remote_usernames.clear();
        self.optimistic_room_playlist = None;
    }

    fn current_room_name(&self) -> Option<&str> {
        self.runtime
            .session()
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(super) fn projected_current_room_playlist(&self) -> Option<&RoomPlaylistView> {
        if let Some(playlist) = self.runtime.session().current_room_playlist() {
            return Some(playlist);
        }
        let current_room = self.current_room_name()?;
        self.optimistic_room_playlist
            .as_ref()
            .and_then(|(room_name, playlist)| (room_name == current_room).then_some(playlist))
    }

    fn sync_optimistic_room_playlist(&mut self) {
        if self.runtime.session().current_room_playlist().is_some() {
            self.optimistic_room_playlist = None;
            return;
        }

        let current_room = self.current_room_name();
        if self
            .optimistic_room_playlist
            .as_ref()
            .is_some_and(|(room_name, _)| Some(room_name.as_str()) != current_room)
        {
            self.optimistic_room_playlist = None;
        }
    }

    fn set_optimistic_current_room_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) {
        let Some(room_name) = self.current_room_name().map(str::to_owned) else {
            self.optimistic_room_playlist = None;
            return;
        };

        let index = if files.is_empty() {
            None
        } else {
            selected_index
                .filter(|index| *index < files.len())
                .and_then(|index| i64::try_from(index).ok())
                .or(Some(0))
        };
        self.optimistic_room_playlist = Some((
            room_name,
            RoomPlaylistView {
                files,
                index,
                set_by: Some(self.username.clone()),
            },
        ));
    }

    fn queue_periodic_state_sync_heartbeat_if_due(&mut self) {
        if self.runtime.session().server_chat_supported().is_none() {
            self.next_state_sync_heartbeat_at = None;
            return;
        }

        let now = Instant::now();
        let Some(next_heartbeat_at) = self.next_state_sync_heartbeat_at else {
            self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
            return;
        };
        if now < next_heartbeat_at {
            return;
        }

        let _ = self
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible();
        self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
    }

    fn autoplay_runtime_flags(&self) -> (bool, bool, bool, bool) {
        let session = self.runtime.session();
        let readiness_supported = session.server_readiness_supported().unwrap_or(false);
        let local_can_control = session.local_can_control().unwrap_or(false);
        let is_playing_music = session.is_playing_music();
        let recently_advanced = session.recently_advanced(system_time_seconds());
        (
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        )
    }

    fn sync_autoplay_runtime(&mut self, actions: &mut Vec<GuiShellAction>) {
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );

        if !self.runtime.session().autoplay_timer_is_running() {
            self.next_autoplay_tick_at = None;
            return;
        }

        let tick_interval =
            Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS.max(f64::EPSILON));
        let now = Instant::now();
        let Some(next_autoplay_tick_at) = self.next_autoplay_tick_at else {
            self.next_autoplay_tick_at = Some(now + tick_interval);
            return;
        };
        if now < next_autoplay_tick_at {
            return;
        }

        if let Err(error) = self.runtime.tick_autoplay(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core autoplay dispatch failed: {error}"),
            });
            self.next_autoplay_tick_at = None;
            return;
        }

        self.next_autoplay_tick_at = if self.runtime.session().autoplay_timer_is_running() {
            Some(now + tick_interval)
        } else {
            None
        };
    }

    pub(super) fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        let mut lines: Vec<_> = self.pending_startup_protocol_lines.drain(..).collect();
        lines.extend(
            self.runtime
                .flush_queued_protocol_lines()
                .map_err(|error| format!("Queued protocol line encoding failed: {error}"))?,
        );
        Ok(lines)
    }

    pub(super) fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        let message = decode_message_line(json_line)
            .map_err(|error| format!("Inbound client-session message decode failed: {error}"))?;
        let result = match message {
            ProtocolMessage::State(state_message) => {
                let _ = self
                    .runtime
                    .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
                        state_message.state,
                    );
                Ok(())
            }
            other => self
                .runtime
                .session_mut()
                .apply_protocol_message(other)
                .map_err(|error| format!("Inbound client-session message apply failed: {error}")),
        };
        self.sync_optimistic_room_playlist();
        result
    }
}

impl GuiSessionRuntimeAdapter for GuiClientCoreChatSessionRuntimeAdapter {
    fn drain_gui_actions(&mut self, state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        let mut trailing_actions = Vec::new();
        let language = Some(state.runtime_language_tag_legacy_compatible());
        if let Err(error) = self.runtime.run_user_change_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core user-change dispatch failed: {error}"),
            });
        } else {
            for notification in self.runtime.drain_user_change_notifications() {
                self.note_user_change(notification.clone());
                if let Some(action) = Self::user_change_action(notification, language) {
                    trailing_actions.push(action);
                }
            }
        }
        if let Err(error) = self.runtime.run_reconnect_transition_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect transition dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::reconnect_transition_actions(notification, language)
                    }),
            );
        }
        if let Err(error) = self.runtime.run_reconnect_state_restore_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect state-restore dispatch failed: {error}"),
            });
        }
        if let Err(error) = self
            .runtime
            .run_reconnect_state_restore_validation_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect validation dispatch failed: {error}"),
            });
        }
        if !actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    ..
                }
            )
        }) {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::reconnect_transition_actions(notification, language)
                    }),
            );
        } else {
            self.runtime.drain_reconnect_notifications();
        }
        if let Err(error) = self
            .runtime
            .run_controlled_room_creation_notifications_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controlled-room dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controlled_room_creation_notifications()
                    .into_iter()
                    .flat_map(Self::controlled_room_creation_action),
            );
        }
        if let Err(error) = self.runtime.run_controller_reidentify_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller reidentify dispatch failed: {error}"),
            });
        }
        if let Err(error) = self.runtime.run_controller_auth_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller-auth dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controller_auth_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::controller_auth_transition_action(notification, language)
                    }),
            );
        }
        self.sync_autoplay_runtime(&mut actions);
        trailing_actions.extend(
            self.runtime
                .drain_autoplay_notifications()
                .into_iter()
                .flat_map(Self::autoplay_countdown_action),
        );
        if let Err(error) = self.runtime.run_chat_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core chat notification dispatch failed: {error}"),
            });
        }
        self.queue_periodic_state_sync_heartbeat_if_due();

        let main_window_runtime_snapshot = self.main_window_runtime_snapshot(state);
        let interaction_runtime_snapshot = self.interaction_runtime_snapshot(
            state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.playlist.len())
                .unwrap_or_else(|| state.main_window.playlist.len()),
        );
        let menu_dialog_runtime_snapshot = self.menu_dialog_runtime_snapshot(
            state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.shared_playlist_enabled)
                .unwrap_or(state.main_window.shared_playlist_enabled),
        );
        if let Some(snapshot) = main_window_runtime_snapshot
            && snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window)
        {
            actions.push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = interaction_runtime_snapshot {
            actions.push(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = menu_dialog_runtime_snapshot {
            actions.push(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot));
        }

        actions.extend(
            self.runtime
                .drain_chat_notifications()
                .into_iter()
                .map(|notification| match notification {
                    ChatNotification::Message { username, message } => {
                        GuiShellAction::PushChatMessage {
                            sender: username.unwrap_or_else(|| "Server".to_owned()),
                            message,
                        }
                    }
                }),
        );
        actions.extend(trailing_actions);
        actions
    }

    fn adjust_command_availability(
        &self,
        _state: &SyncplayGuiShellAppState,
        mut command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        if self.runtime.session().server_chat_supported() != Some(true) {
            command_availability.can_send_chat_message = false;
        }
        command_availability
    }

    fn playlist_control_available(&self) -> bool {
        self.shared_playlist_control_available()
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        GuiClientCoreChatSessionRuntimeAdapter::flush_outbound_protocol_lines(self)
    }

    fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        GuiClientCoreChatSessionRuntimeAdapter::apply_message_json(self, json_line)
    }

    fn set_room(&mut self, room: String) -> Result<(), String> {
        match self.runtime.run_set_room(room) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        match self.runtime.run_set_room_with_legacy_fallback(default_room) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String> {
        match self.runtime.run_send_chat_message(message) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_chat_supported() {
                None => Err(
                    "Client-core session runtime cannot send chat until the server Hello enables chat."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot send chat because the server disabled chat."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound chat message."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime chat dispatch failed: {error}"
            )),
        }
    }

    fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user("", ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change readiness until the server Hello enables readiness."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change readiness because the server disabled readiness."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound readiness change."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn mark_local_media_opened_not_ready(&mut self) -> Result<bool, String> {
        self.runtime
            .run_local_media_opened_not_ready()
            .map_err(|error| {
                format!(
                    "Client-core session runtime local media-open readiness dispatch failed: {error}"
                )
            })
    }

    fn set_user_ready(&mut self, username: String, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user(username, ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_set_others_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change other users' readiness until the server Hello enables remote readiness changes."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change other users' readiness because the server disabled remote readiness changes."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound remote readiness change."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn request_controller_auth(&mut self, room: String, password: String) -> Result<(), String> {
        match self.runtime.run_request_controller_auth(room, password) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().username.is_none() {
                    Err(
                        "Client-core session runtime cannot request controller access until the server Hello is received."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound controller-auth request."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime controller-auth dispatch failed: {error}"
            )),
        }
    }

    fn queue_playlist_entry(
        &mut self,
        entry: String,
        select_after_queue: bool,
    ) -> Result<(), String> {
        match self
            .runtime
            .run_queue_playlist_item(entry, select_after_queue)
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist entry."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist queue dispatch failed: {error}"
            )),
        }
    }

    fn set_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_set_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist selection before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist selection change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist selection dispatch failed: {error}"
            )),
        }
    }

    fn advance_playlist_index(&mut self) -> Result<(), String> {
        match self.runtime.run_advance_playlist_index() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot advance the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist advancement."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist advancement dispatch failed: {error}"
            )),
        }
    }

    fn delete_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_delete_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot remove shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist removal."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist removal dispatch failed: {error}"
            )),
        }
    }

    fn replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<(), String> {
        match self
            .runtime
            .run_replace_playlist(files.clone(), selected_index)
        {
            Ok(true) => {
                self.set_optimistic_current_room_playlist(files, selected_index);
                Ok(())
            }
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot reorder the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist reorder."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist reorder dispatch failed: {error}"
            )),
        }
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        match self.runtime.run_undo_playlist_change() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot undo shared playlist changes before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist undo."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist undo dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_remaining_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle remaining shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_entire_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn sync_local_playback_telemetry(
        &mut self,
        paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .apply_player_playback_telemetry_update(&PlayerPlaybackTelemetryUpdate {
                paused,
                position_seconds,
                playback_rate: None,
            });
        Ok(())
    }

    fn set_playback_paused(&mut self, paused: bool) -> Result<bool, String> {
        match self.runtime.run_set_paused(paused) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime playback pause dispatch failed: {error}"
            )),
        }
    }

    fn record_manual_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, String> {
        match self.runtime.run_seek_to_position(position_seconds) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime seek dispatch failed: {error}"
            )),
        }
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        match self.runtime.run_undo_seek() {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime undo-seek dispatch failed: {error}"
            )),
        }
    }

    fn local_position_seconds(&self) -> Option<f64> {
        self.runtime.session().local_position_seconds()
    }

    fn local_username(&self) -> Option<&str> {
        self.runtime.session().username.as_deref()
    }

    fn current_room_playstate(&self) -> Option<GuiSessionRoomPlaystate> {
        self.runtime
            .session()
            .current_room_playstate()
            .map(|playstate| GuiSessionRoomPlaystate {
                position_seconds: playstate.position,
                paused: playstate.paused,
                do_seek: playstate.do_seek,
                set_by: playstate.set_by.clone(),
            })
    }

    fn current_room_playstate_for_attached_player_sync(&self) -> Option<GuiSessionRoomPlaystate> {
        self.runtime
            .current_room_playstate_legacy_ping_compatible_now()
            .map(|playstate| GuiSessionRoomPlaystate {
                position_seconds: playstate.position,
                paused: playstate.paused,
                do_seek: playstate.do_seek,
                set_by: playstate.set_by,
            })
    }

    fn note_local_playlist_index_reset_intent(&mut self, pause_before_sync: bool) {
        self.runtime.session_mut().begin_local_playlist_index_reset_intent(
            pause_before_sync,
            system_time_seconds(),
        );
    }

    fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        self.runtime
            .session_mut()
            .take_pending_playlist_index_reset_intent()
    }

    fn has_pending_playlist_index_reset_intent(&self) -> bool {
        self.runtime
            .session()
            .has_pending_playlist_index_reset_intent()
    }

    fn set_autoplay_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.runtime.session_mut().set_autoplay_enabled(enabled);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, threshold: usize) -> Result<(), String> {
        self.runtime
            .session_mut()
            .readiness_autoplay_config_mut()
            .auto_play_threshold = Some(threshold);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), String> {
        self.runtime
            .publish_local_file_legacy_compatible(
                file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            )
            .map_err(|error| {
                format!("Client-core session runtime local file publish failed: {error}")
            })
    }

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        let Some((_label, address)) = selected_server else {
            return Err(
                "Client-core session runtime cannot connect because no public server is selected."
                    .to_owned(),
            );
        };
        let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
        if host.trim().is_empty() {
            return Err(
                "Client-core session runtime cannot connect because the selected public-server address is invalid."
                    .to_owned(),
            );
        }
        self.reset_session_for_reconnect();
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        _current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(refreshed_servers) = Self::refreshed_public_server_rows_from_env()? {
            return Ok(refreshed_servers);
        }
        #[cfg(test)]
        {
            Ok(Self::normalize_public_server_rows(_current_servers))
        }
        #[cfg(not(test))]
        {
            let refreshed_servers = remote_services::fetch_public_servers(_language)?;
            Ok(Self::normalize_public_server_rows(refreshed_servers))
        }
    }

    fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        GuiClientCoreChatSessionRuntimeAdapter::missing_media_search_target_file_name(self)
    }

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String> {
        let target_file_name = self.missing_media_search_target_file_name()?;
        for directory in directories {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                Self::search_path_for_missing_media_target(&target_file_name, Path::new(trimmed))?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }
}
