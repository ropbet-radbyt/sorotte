use serde_json::Value;
pub use sorotte_client_core::ConnectionPhase;
use sorotte_client_core::{
    AutoplayCountdownNotification, ChatNotification, ClientEffect, ClientEffectError,
    ClientPlayerIo, ClientRuntime, ClientSession, ClientSessionUpdate,
    ControlledRoomCreationNotification, ControllerAuthTransitionNotification, FileSize,
    PrivacyMode, QueuedRuntimeControl, ReconnectStateRestoreCorrectionMetrics,
    ReconnectStateRestoreCorrectionStateSnapshot, ReconnectTransitionNotification,
    RoomPlaystateView, UserChangeNotification,
};
use sorotte_player_api::{PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate};
pub use sorotte_plex::PlexClientConfig;
use sorotte_plex::{PlexHttpClient, PlexMatchCache, PlexSyncEngine, PlexWatchEvent};
use sorotte_protocol::{ProtocolError, ProtocolMessage, StatePayload, decode_message_line_items};
use sorotte_secret::SecretValue;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const PLEX_SYNC_PUMP_INTERVAL: Duration = Duration::from_secs(1);

type ApplicationPlexSyncEngine = PlexSyncEngine<PlexHttpClient>;

struct ClientPlexService {
    engine: Option<ApplicationPlexSyncEngine>,
    worker: Option<tokio::task::JoinHandle<ClientPlexWorkerResult>>,
    cache_path: Option<PathBuf>,
    next_tick_due_at: Option<Instant>,
}

struct ClientPlexWorkerResult {
    engine: ApplicationPlexSyncEngine,
    cache_save_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientApplicationSettings {
    pub autoplay_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientCommand {
    Connect {
        endpoint: String,
    },
    TransportConnected,
    Reconnect {
        attempt: u32,
    },
    Disconnect {
        now_seconds: f64,
    },
    ReceiveProtocolLine {
        line: String,
        received_at_seconds: f64,
    },
    SetRoom {
        room: String,
        legacy_fallback: bool,
    },
    SetReady {
        username: Option<String>,
        ready: Option<bool>,
        manually_initiated: bool,
    },
    OpenMedia {
        path: String,
    },
    PlayerPlaybackObserved {
        paused: bool,
        position_seconds: f64,
    },
    UpdateSettings(ClientApplicationSettings),
    SendChat(String),
    RequestUserList,
    SetPlaylistIndex(i64),
    AdvancePlaylistIndex,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    SeekToPosition(f64),
    SeekByOffset(f64),
    TogglePause,
    SetPaused(bool),
    RequestControllerAuth {
        room: String,
        password: SecretValue,
    },
}

impl ClientCommand {
    pub fn request_controller_auth(room: impl Into<String>, password: impl Into<String>) -> Self {
        Self::RequestControllerAuth {
            room: room.into(),
            password: SecretValue::new(password),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    ConnectionChanged(ConnectionPhase),
    RoomChanged {
        previous: Option<String>,
        current: Option<String>,
    },
    PlaybackChanged {
        paused: Option<bool>,
        position_seconds: Option<f64>,
    },
    ReadinessChanged {
        username: Option<String>,
        ready: Option<bool>,
    },
    CommandCompleted {
        command: &'static str,
        changed: bool,
    },
    Notification(String),
    OperationFailed {
        operation: &'static str,
        message: String,
    },
}

impl ClientEvent {
    pub fn command_changed(&self) -> Option<bool> {
        match self {
            Self::CommandCompleted { changed, .. } => Some(*changed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ApplicationSnapshot {
    connection_phase: ConnectionPhase,
    room: Option<String>,
    paused: Option<bool>,
    position_seconds: Option<f64>,
    username: Option<String>,
    ready: Option<bool>,
}

pub struct ClientApplication<P>
where
    P: PlayerAdapter,
{
    runtime: ClientRuntime<P, QueuedRuntimeControl>,
    endpoint: Option<String>,
    plex: Option<ClientPlexService>,
}

impl<P> ClientApplication<P>
where
    P: PlayerAdapter,
{
    pub fn with_default_session(player: P) -> Self {
        Self::new(ClientSession::default(), player)
    }

    pub fn new(session: ClientSession, player: P) -> Self {
        Self::from_runtime(ClientRuntime::new(
            session,
            player,
            QueuedRuntimeControl::default(),
        ))
    }

    pub fn from_runtime(runtime: ClientRuntime<P, QueuedRuntimeControl>) -> Self {
        Self {
            runtime,
            endpoint: None,
            plex: None,
        }
    }

    pub fn into_runtime(self) -> ClientRuntime<P, QueuedRuntimeControl> {
        self.runtime
    }

    pub fn connection_phase(&self) -> &ConnectionPhase {
        self.runtime.session().connection_phase()
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    pub fn plex_service_enabled(&self) -> bool {
        self.plex.is_some()
    }

    pub async fn configure_plex_service(
        &mut self,
        config: &PlexClientConfig,
        client_identifier: &str,
        cache_path: Option<PathBuf>,
    ) -> Vec<ClientEvent> {
        let mut events = self.shutdown_plex_service().await;
        if !config.enabled || !config.has_selected_server() {
            return events;
        }

        let config = config.clone();
        let client_identifier = client_identifier.to_owned();
        match tokio::task::spawn_blocking(move || {
            let client = PlexHttpClient::new(&client_identifier)?;
            let (cache, cache_load_error) = load_plex_match_cache(cache_path.as_deref());
            Ok::<_, sorotte_plex::PlexError>((
                ClientPlexService {
                    engine: Some(PlexSyncEngine::new(config, client, cache)),
                    worker: None,
                    cache_path,
                    next_tick_due_at: None,
                },
                cache_load_error,
            ))
        })
        .await
        {
            Ok(Ok((service, cache_load_error))) => {
                self.plex = Some(service);
                if let Some(message) = cache_load_error {
                    events.push(ClientEvent::Notification(message));
                }
            }
            Ok(Err(error)) => events.push(ClientEvent::OperationFailed {
                operation: "configure-plex",
                message: format!("failed to initialize Plex sync client: {error}"),
            }),
            Err(error) => events.push(ClientEvent::OperationFailed {
                operation: "configure-plex",
                message: format!("Plex sync worker failed during initialization: {error}"),
            }),
        }
        events
    }

    pub async fn pump_plex_service(&mut self) -> Vec<ClientEvent> {
        let watch_event = self.current_plex_watch_event();
        let Some(plex) = self.plex.as_mut() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if plex
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let worker = plex
                .worker
                .take()
                .expect("finished worker should be present");
            match worker.await {
                Ok(result) => {
                    plex.engine = Some(result.engine);
                    if let Some(message) = result.cache_save_error {
                        events.push(ClientEvent::Notification(message));
                    }
                }
                Err(error) => events.push(ClientEvent::OperationFailed {
                    operation: "pump-plex",
                    message: format!("Plex sync worker failed: {error}"),
                }),
            }
        }
        if plex.worker.is_some() {
            return events;
        }
        let now = Instant::now();
        if plex.next_tick_due_at.is_some_and(|due_at| now < due_at) {
            return events;
        }
        let Some(mut engine) = plex.engine.take() else {
            return events;
        };
        let cache_path = plex.cache_path.clone();
        plex.worker = Some(tokio::task::spawn_blocking(move || {
            let before = engine.cache().clone();
            let _ = engine.tick(watch_event, SystemTime::now());
            let cache_save_error =
                plex_cache_save_error_if_changed(&engine, cache_path.as_deref(), &before);
            ClientPlexWorkerResult {
                engine,
                cache_save_error,
            }
        }));
        plex.next_tick_due_at = Some(now + PLEX_SYNC_PUMP_INTERVAL);
        events
    }

    pub async fn shutdown_plex_service(&mut self) -> Vec<ClientEvent> {
        let Some(mut plex) = self.plex.take() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if let Some(worker) = plex.worker.take() {
            match worker.await {
                Ok(result) => {
                    plex.engine = Some(result.engine);
                    if let Some(message) = result.cache_save_error {
                        events.push(ClientEvent::Notification(message));
                    }
                }
                Err(error) => events.push(ClientEvent::OperationFailed {
                    operation: "shutdown-plex",
                    message: format!("Plex sync worker failed before final stop: {error}"),
                }),
            }
        }
        let Some(mut engine) = plex.engine.take() else {
            return events;
        };
        let cache_path = plex.cache_path.take();
        match tokio::task::spawn_blocking(move || {
            let before = engine.cache().clone();
            let _ = engine.tick(None, SystemTime::now());
            plex_cache_save_error_if_changed(&engine, cache_path.as_deref(), &before)
        })
        .await
        {
            Ok(Some(message)) => events.push(ClientEvent::Notification(message)),
            Ok(None) => {}
            Err(error) => events.push(ClientEvent::OperationFailed {
                operation: "shutdown-plex",
                message: format!("Plex final sync worker failed: {error}"),
            }),
        }
        events
    }

    fn current_plex_watch_event(&self) -> Option<PlexWatchEvent> {
        let session = self.runtime.session();
        let username = session.username()?;
        if session.user_has_file(username) == Some(false) {
            return None;
        }
        let mut session_file =
            sorotte_player_api::LocalFileUpdate::new(session.user_file_name(username)?.to_owned());
        session_file.duration_seconds = session.user_file_duration(username);
        session_file.size_bytes = session.user_file_size(username).and_then(FileSize::as_u64);
        let mut file = self
            .runtime
            .last_local_file_update()
            .cloned()
            .unwrap_or_else(|| session_file.clone());
        if file.duration_seconds.is_none() {
            file.duration_seconds = session_file.duration_seconds;
        }
        if file.size_bytes.is_none() {
            file.size_bytes = session_file.size_bytes;
        }
        let mut event = PlexWatchEvent::new(file).with_changed_at(SystemTime::now());
        if let Some(position_seconds) = session.local_position_seconds() {
            event = event.with_position_seconds(position_seconds);
        }
        if let Some(duration_seconds) = event.duration_seconds {
            event = event.with_duration_seconds(duration_seconds);
        }
        if let Some(paused) = session.local_paused() {
            event = event.with_paused(paused);
        }
        Some(event)
    }

    pub fn session(&self) -> &ClientSession {
        self.runtime.session()
    }

    pub fn session_mut(&mut self) -> ClientSessionUpdate<'_> {
        self.runtime.session_mut()
    }

    pub fn player(&self) -> &P {
        self.runtime.player()
    }

    pub fn player_mut(&mut self) -> ClientPlayerIo<'_, P> {
        self.runtime.player_mut()
    }

    pub fn with_player_io<R>(&mut self, io: impl FnOnce(&mut P) -> R) -> R {
        self.runtime.with_player_io(io)
    }

    fn snapshot(&self) -> ApplicationSnapshot {
        let session = self.runtime.session();
        let username = session.username().map(ToOwned::to_owned);
        ApplicationSnapshot {
            connection_phase: session.connection_phase().clone(),
            room: session.room().map(ToOwned::to_owned),
            paused: session.local_paused(),
            position_seconds: session.local_position_seconds(),
            ready: username
                .as_deref()
                .and_then(|username| session.user_ready(username)),
            username,
        }
    }

    fn domain_events_since(&self, before: &ApplicationSnapshot) -> Vec<ClientEvent> {
        let after = self.snapshot();
        let mut events = Vec::new();
        if before.connection_phase != after.connection_phase {
            events.push(ClientEvent::ConnectionChanged(
                after.connection_phase.clone(),
            ));
        }
        if before.room != after.room {
            events.push(ClientEvent::RoomChanged {
                previous: before.room.clone(),
                current: after.room.clone(),
            });
        }
        if before.paused != after.paused || before.position_seconds != after.position_seconds {
            events.push(ClientEvent::PlaybackChanged {
                paused: after.paused,
                position_seconds: after.position_seconds,
            });
        }
        if before.username != after.username || before.ready != after.ready {
            events.push(ClientEvent::ReadinessChanged {
                username: after.username,
                ready: after.ready,
            });
        }
        events
    }

    fn set_connection_phase(&mut self, phase: ConnectionPhase) -> Vec<ClientEvent> {
        if self.connection_phase() == &phase {
            return Vec::new();
        }
        let mut session = self.runtime.session_mut();
        match phase {
            ConnectionPhase::Disconnected => session.mark_disconnected(),
            ConnectionPhase::Connecting => session.mark_connecting(),
            ConnectionPhase::AwaitingHello => session.mark_awaiting_hello(),
            ConnectionPhase::Reconnecting { attempt } => session.mark_reconnecting(attempt),
            ConnectionPhase::Closing => session.mark_closing(),
            ConnectionPhase::Active(_) => {
                unreachable!("Active is entered only by applying a server Hello")
            }
        }
        vec![ClientEvent::ConnectionChanged(
            self.connection_phase().clone(),
        )]
    }

    pub fn dispatch(&mut self, command: ClientCommand) -> Vec<ClientEvent> {
        let before = self.snapshot();
        let (operation, result) = match command {
            ClientCommand::Connect { endpoint } => {
                self.endpoint = Some(endpoint);
                return self.set_connection_phase(ConnectionPhase::Connecting);
            }
            ClientCommand::TransportConnected => {
                return self.set_connection_phase(ConnectionPhase::AwaitingHello);
            }
            ClientCommand::Reconnect { attempt } => {
                return self.set_connection_phase(ConnectionPhase::Reconnecting { attempt });
            }
            ClientCommand::Disconnect { now_seconds } => {
                self.runtime.session_mut().mark_closing();
                (
                    "disconnect",
                    self.runtime.run_disconnect(now_seconds).map(|()| true),
                )
            }
            ClientCommand::ReceiveProtocolLine {
                line,
                received_at_seconds,
            } => (
                "receive-protocol-line",
                self.runtime
                    .session_mut()
                    .apply_message_json_at(&line, received_at_seconds)
                    .map(|()| true)
                    .map_err(protocol_player_error),
            ),
            ClientCommand::SetRoom {
                room,
                legacy_fallback,
            } => (
                "set-room",
                if legacy_fallback {
                    self.runtime.run_set_room_with_legacy_fallback(room)
                } else {
                    self.runtime.run_set_room(room)
                },
            ),
            ClientCommand::SetReady {
                username,
                ready,
                manually_initiated,
            } => (
                "set-ready",
                match (username, ready) {
                    (Some(username), Some(ready)) => {
                        self.runtime
                            .run_set_ready_for_user(username, ready, manually_initiated)
                    }
                    (None, Some(ready)) => {
                        self.runtime
                            .run_set_ready_for_user("", ready, manually_initiated)
                    }
                    (_, None) => self.runtime.run_toggle_ready(manually_initiated),
                },
            ),
            ClientCommand::OpenMedia { path } => (
                "open-media",
                self.runtime.player_mut().open_file(&path).map(|()| true),
            ),
            ClientCommand::PlayerPlaybackObserved {
                paused,
                position_seconds,
            } => {
                self.runtime
                    .session_mut()
                    .apply_player_playback_telemetry_update(
                        &PlayerPlaybackTelemetryUpdate::default()
                            .with_paused(paused)
                            .with_position_seconds(position_seconds),
                    );
                ("player-playback-observed", Ok(true))
            }
            ClientCommand::UpdateSettings(settings) => {
                if let Some(enabled) = settings.autoplay_enabled {
                    self.runtime.session_mut().set_autoplay_enabled(enabled);
                }
                ("update-settings", Ok(true))
            }
            ClientCommand::SendChat(message) => {
                ("send-chat", self.runtime.run_send_chat_message(message))
            }
            ClientCommand::RequestUserList => {
                ("request-user-list", self.runtime.run_request_user_list())
            }
            ClientCommand::SetPlaylistIndex(index) => (
                "set-playlist-index",
                self.runtime.run_set_playlist_index(index),
            ),
            ClientCommand::AdvancePlaylistIndex => (
                "advance-playlist-index",
                self.runtime.run_advance_playlist_index(),
            ),
            ClientCommand::QueuePlaylistItem {
                file_name,
                select_after_queue,
            } => (
                "queue-playlist-item",
                self.runtime
                    .run_queue_playlist_item(file_name, select_after_queue),
            ),
            ClientCommand::DeletePlaylistIndex(index) => (
                "delete-playlist-index",
                self.runtime.run_delete_playlist_index(index),
            ),
            ClientCommand::UndoPlaylistChange => (
                "undo-playlist-change",
                self.runtime.run_undo_playlist_change(),
            ),
            ClientCommand::ShuffleRemainingPlaylist => (
                "shuffle-remaining-playlist",
                self.runtime.run_shuffle_remaining_playlist(),
            ),
            ClientCommand::ShuffleEntirePlaylist => (
                "shuffle-entire-playlist",
                self.runtime.run_shuffle_entire_playlist(),
            ),
            ClientCommand::UndoSeek => ("undo-seek", self.runtime.run_undo_seek()),
            ClientCommand::SeekToPosition(position_seconds) => (
                "seek-to-position",
                self.runtime.run_seek_to_position(position_seconds),
            ),
            ClientCommand::SeekByOffset(offset_seconds) => (
                "seek-by-offset",
                self.runtime.run_seek_by_offset(offset_seconds),
            ),
            ClientCommand::TogglePause => ("toggle-pause", self.runtime.run_toggle_pause()),
            ClientCommand::SetPaused(paused) => ("set-paused", self.runtime.run_set_paused(paused)),
            ClientCommand::RequestControllerAuth { room, password } => (
                "request-controller-auth",
                self.runtime
                    .run_request_controller_auth(room, password.expose_secret()),
            ),
        };

        match result {
            Ok(changed) => {
                let mut events = self.domain_events_since(&before);
                events.push(ClientEvent::CommandCompleted {
                    command: operation,
                    changed,
                });
                events
            }
            Err(error) => vec![ClientEvent::OperationFailed {
                operation,
                message: error.to_string(),
            }],
        }
    }

    pub fn pending_protocol_line(&self) -> Result<Option<String>, ProtocolError> {
        self.runtime.pending_protocol_line()
    }

    /// Decodes a transport line and applies its domain messages in wire order.
    /// Returns whether an inbound state message emitted an immediate state-sync response.
    pub fn apply_protocol_line(
        &mut self,
        line: &str,
        received_at_seconds: f64,
        reconcile_inbound_state: bool,
        dont_slow_down_with_me: bool,
        apply_fallback_json: bool,
    ) -> Result<bool, ProtocolError> {
        let messages = decode_message_line_items(line)?;
        if messages.is_empty() {
            if apply_fallback_json {
                self.runtime
                    .session_mut()
                    .apply_message_json_at(line, received_at_seconds)?;
            }
            return Ok(false);
        }

        let mut state_sync_emitted = false;
        for item in messages {
            let Ok(message) = item.message else {
                break;
            };
            match message {
                ProtocolMessage::State(state) if reconcile_inbound_state => {
                    state_sync_emitted |= self
                        .runtime
                        .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
                            state.state,
                            dont_slow_down_with_me,
                        );
                }
                other => self
                    .runtime
                    .session_mut()
                    .apply_protocol_message_at(other, received_at_seconds)?,
            }
        }
        Ok(state_sync_emitted)
    }

    pub fn acknowledge_protocol_line(&mut self) -> Option<ProtocolMessage> {
        self.runtime.acknowledge_protocol_line()
    }

    pub fn pending_protocol_message_count(&self) -> usize {
        self.runtime.control().outbound_messages().len()
    }

    pub fn pending_protocol_messages(&self) -> &VecDeque<ProtocolMessage> {
        self.runtime.control().outbound_messages()
    }

    pub fn flush_queued_protocol_lines_to_transport<F>(
        &mut self,
        send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        self.runtime
            .flush_queued_protocol_lines_to_transport(send_line)
    }

    pub fn last_local_file_update(&self) -> Option<&sorotte_player_api::LocalFileUpdate> {
        self.runtime.last_local_file_update()
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        self.runtime.reconnect_state_restore_correction_metrics()
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        self.runtime
            .reconnect_state_restore_correction_state_snapshot()
    }

    pub fn drain_player_playback_telemetry_updates(
        &mut self,
    ) -> Vec<PlayerPlaybackTelemetryUpdate> {
        self.runtime.drain_player_playback_telemetry_updates()
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.runtime.drain_autoplay_notifications()
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.runtime.drain_chat_notifications()
    }

    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        self.runtime.drain_controlled_room_creation_notifications()
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.runtime.drain_controller_auth_notifications()
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.runtime.drain_user_change_notifications()
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.runtime.drain_reconnect_notifications()
    }

    pub fn pending_autoplay_notification(&self) -> Option<&AutoplayCountdownNotification> {
        self.runtime.pending_autoplay_notification()
    }

    pub fn acknowledge_autoplay_notification(&mut self) -> Option<AutoplayCountdownNotification> {
        self.runtime.acknowledge_autoplay_notification()
    }

    pub fn drain_autoplay_notifications_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
    where
        F: FnMut(&AutoplayCountdownNotification) -> Result<(), E>,
    {
        self.runtime.drain_autoplay_notifications_to_sink(notify)
    }

    pub fn pending_chat_notification(&self) -> Option<&ChatNotification> {
        self.runtime.pending_chat_notification()
    }

    pub fn acknowledge_chat_notification(&mut self) -> Option<ChatNotification> {
        self.runtime.acknowledge_chat_notification()
    }

    pub fn drain_chat_notifications_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
    where
        F: FnMut(&ChatNotification) -> Result<(), E>,
    {
        self.runtime.drain_chat_notifications_to_sink(notify)
    }

    pub fn pending_controller_auth_notification(
        &self,
    ) -> Option<&ControllerAuthTransitionNotification> {
        self.runtime.pending_controller_auth_notification()
    }

    pub fn acknowledge_controller_auth_notification(
        &mut self,
    ) -> Option<ControllerAuthTransitionNotification> {
        self.runtime.acknowledge_controller_auth_notification()
    }

    pub fn drain_controller_auth_notifications_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
    where
        F: FnMut(&ControllerAuthTransitionNotification) -> Result<(), E>,
    {
        self.runtime
            .drain_controller_auth_notifications_to_sink(notify)
    }

    pub fn pending_user_change_notification(&self) -> Option<&UserChangeNotification> {
        self.runtime.pending_user_change_notification()
    }

    pub fn acknowledge_user_change_notification(&mut self) -> Option<UserChangeNotification> {
        self.runtime.acknowledge_user_change_notification()
    }

    pub fn drain_user_change_notifications_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
    where
        F: FnMut(&UserChangeNotification) -> Result<(), E>,
    {
        self.runtime.drain_user_change_notifications_to_sink(notify)
    }

    pub fn pending_reconnect_notification(&self) -> Option<&ReconnectTransitionNotification> {
        self.runtime.pending_reconnect_notification()
    }

    pub fn acknowledge_reconnect_notification(
        &mut self,
    ) -> Option<ReconnectTransitionNotification> {
        self.runtime.acknowledge_reconnect_notification()
    }

    pub fn drain_reconnect_notifications_to_sink<F, E>(&mut self, notify: F) -> Result<(), E>
    where
        F: FnMut(&ReconnectTransitionNotification) -> Result<(), E>,
    {
        self.runtime.drain_reconnect_notifications_to_sink(notify)
    }

    pub fn run_player_chat_input_if_needed(&mut self) -> Result<usize, PlayerError> {
        self.runtime.run_player_chat_input_if_needed()
    }

    pub fn run_send_chat_message(
        &mut self,
        message: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        self.runtime.run_send_chat_message(message)
    }

    pub fn run_set_ready_for_user(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .run_set_ready_for_user(username, ready, manually_initiated)
    }

    pub fn run_request_user_list(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_request_user_list()
    }

    pub fn run_set_room(&mut self, room: impl Into<String>) -> Result<bool, PlayerError> {
        self.runtime.run_set_room(room)
    }

    pub fn run_set_room_with_legacy_fallback(
        &mut self,
        room: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        self.runtime.run_set_room_with_legacy_fallback(room)
    }

    pub fn run_local_media_opened_not_ready(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_local_media_opened_not_ready()
    }

    pub fn run_request_controller_auth(
        &mut self,
        room: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        self.runtime.run_request_controller_auth(room, password)
    }

    pub fn run_set_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        self.runtime.run_set_playlist_index(index)
    }

    pub fn run_advance_playlist_index(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_advance_playlist_index()
    }

    pub fn run_queue_playlist_item(
        &mut self,
        file_name: impl Into<String>,
        select_after_queue: bool,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .run_queue_playlist_item(file_name, select_after_queue)
    }

    pub fn run_delete_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        self.runtime.run_delete_playlist_index(index)
    }

    pub fn run_replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<bool, PlayerError> {
        self.runtime.run_replace_playlist(files, selected_index)
    }

    pub fn run_undo_playlist_change(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_undo_playlist_change()
    }

    pub fn run_shuffle_remaining_playlist(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_shuffle_remaining_playlist()
    }

    pub fn run_shuffle_entire_playlist(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_shuffle_entire_playlist()
    }

    pub fn run_set_paused(&mut self, paused: bool) -> Result<bool, PlayerError> {
        self.runtime.run_set_paused(paused)
    }

    pub fn run_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, PlayerError> {
        self.runtime.run_seek_to_position(position_seconds)
    }

    pub fn run_undo_seek(&mut self) -> Result<bool, PlayerError> {
        self.runtime.run_undo_seek()
    }

    pub fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), PlayerError> {
        self.runtime.publish_local_file_legacy_compatible(
            file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        )
    }

    pub fn publish_pending_local_file_update_legacy_compatible(
        &mut self,
        filename_privacy_mode: sorotte_client_core::PrivacyMode,
        filesize_privacy_mode: sorotte_client_core::PrivacyMode,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .publish_pending_local_file_update_legacy_compatible(
                filename_privacy_mode,
                filesize_privacy_mode,
            )
    }

    pub fn run_disconnect(&mut self, now_seconds: f64) -> Result<(), PlayerError> {
        self.runtime.run_disconnect(now_seconds)
    }

    pub fn run_reconnect_retry(&mut self, retries: u32) -> Result<(), PlayerError> {
        self.runtime.run_reconnect_retry(retries)
    }

    pub fn drain_reconnect_intents<FS, FT>(&mut self, schedule: FS, stop: FT)
    where
        FS: FnMut(f64),
        FT: FnMut(),
    {
        self.runtime.drain_reconnect_intents(schedule, stop);
    }

    pub fn drain_reconnect_requests(&mut self) -> Vec<f64> {
        self.runtime.drain_reconnect_requests()
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        self.runtime.take_stop_reconnect_requested()
    }

    pub fn run_room_pause_sync_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_room_pause_sync_if_needed()
    }

    pub fn run_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Result<(), PlayerError> {
        self.runtime.run_readiness_unpause_attempt(
            now_seconds,
            readiness_supported,
            local_can_control,
            is_playing_music,
        )
    }

    pub fn update_autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
    }

    pub fn tick_autoplay(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Result<(), PlayerError> {
        self.runtime.tick_autoplay(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        )
    }

    pub fn run_desync_correction_if_needed(
        &mut self,
        now_seconds: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        connected: bool,
    ) -> Result<(), PlayerError> {
        self.runtime.run_desync_correction_if_needed(
            now_seconds,
            local_can_control,
            dont_slow_down_with_me,
            connected,
        )
    }

    pub fn run_reconnect_state_restore_validation_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .run_reconnect_state_restore_validation_if_needed()
    }

    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        &mut self,
        state: StatePayload,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.runtime
            .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
                state,
                dont_slow_down_with_me,
            )
    }

    pub fn run_reconnect_transition_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_reconnect_transition_if_needed()
    }

    pub fn run_controller_reidentify_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_controller_reidentify_if_needed()
    }

    pub fn run_controller_auth_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_controller_auth_notifications_if_needed()
    }

    pub fn run_controlled_room_creation_notifications_if_needed(
        &mut self,
    ) -> Result<(), PlayerError> {
        self.runtime
            .run_controlled_room_creation_notifications_if_needed()
    }

    pub fn run_chat_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_chat_notifications_if_needed()
    }

    pub fn run_user_change_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_user_change_notifications_if_needed()
    }

    pub fn run_reconnect_state_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_reconnect_state_restore_if_needed()
    }

    pub fn run_reconnect_playlist_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_reconnect_playlist_restore_if_needed()
    }

    pub fn current_room_playstate_legacy_ping_compatible_at(
        &self,
        now_seconds: f64,
    ) -> Option<RoomPlaystateView> {
        self.runtime
            .current_room_playstate_legacy_ping_compatible_at(now_seconds)
    }

    pub fn current_room_playstate_legacy_ping_compatible_now(&self) -> Option<RoomPlaystateView> {
        self.runtime
            .current_room_playstate_legacy_ping_compatible_now()
    }

    pub fn run_state_sync_heartbeat_legacy_ping_compatible(
        &mut self,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(dont_slow_down_with_me)
    }

    pub fn flush_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        self.runtime.flush_queued_protocol_lines()
    }

    pub fn emit_effect(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        self.runtime.emit_effect(effect)
    }
}

fn load_plex_match_cache(path: Option<&Path>) -> (PlexMatchCache, Option<String>) {
    let Some(path) = path else {
        return (PlexMatchCache::default(), None);
    };
    match PlexMatchCache::load_from_path(path) {
        Ok(cache) => (cache, None),
        Err(error) => (
            PlexMatchCache::default(),
            Some(format!("failed to load Plex match cache: {error}")),
        ),
    }
}

fn plex_cache_save_error_if_changed(
    engine: &ApplicationPlexSyncEngine,
    cache_path: Option<&Path>,
    before: &PlexMatchCache,
) -> Option<String> {
    if engine.cache() == before {
        return None;
    }
    cache_path.and_then(|path| {
        engine
            .cache()
            .save_to_path(path)
            .err()
            .map(|error| format!("failed to save Plex match cache: {error}"))
    })
}

fn protocol_player_error(error: ProtocolError) -> PlayerError {
    PlayerError::OperationFailed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestPlayer {
        opened: Vec<String>,
        paused: bool,
    }

    impl PlayerAdapter for TestPlayer {
        fn name(&self) -> &'static str {
            "client-application-test"
        }

        fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
            self.opened.push(path.to_owned());
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
            self.paused = paused;
            Ok(())
        }
    }

    #[test]
    fn application_tracks_connection_lifecycle_as_events() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        assert_eq!(
            application.dispatch(ClientCommand::Connect {
                endpoint: "sync.example:8999".to_owned(),
            }),
            vec![ClientEvent::ConnectionChanged(ConnectionPhase::Connecting)],
        );
        assert_eq!(application.endpoint(), Some("sync.example:8999"));
        assert_eq!(
            application.dispatch(ClientCommand::TransportConnected),
            vec![ClientEvent::ConnectionChanged(
                ConnectionPhase::AwaitingHello,
            )],
        );
    }

    #[test]
    fn application_executes_player_commands_and_reports_completion() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let events = application.dispatch(ClientCommand::OpenMedia {
            path: "episode.mkv".to_owned(),
        });

        assert_eq!(application.player().opened, vec!["episode.mkv"]);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::CommandCompleted {
                    command: "open-media",
                    changed: true,
                }
            )
        }));
    }

    #[test]
    fn application_toggles_from_observed_player_playback_state() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved {
            paused: false,
            position_seconds: 12.5,
        });

        let events = application.dispatch(ClientCommand::TogglePause);

        assert!(application.player().paused);
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::CommandCompleted {
                command: "toggle-pause",
                changed: true,
            }
        )));
    }

    #[test]
    fn application_protocol_input_emits_room_change_event() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let events = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.2.255"}}"#
                .to_owned(),
            received_at_seconds: 1.0,
        });

        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::RoomChanged { current, .. } if current.as_deref() == Some("room-a")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::ConnectionChanged(ConnectionPhase::Active(capabilities))
                if !capabilities.chat
        )));
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Active(_)
        ));
    }

    #[test]
    fn application_commands_redact_controller_credentials() {
        let command = ClientCommand::request_controller_auth("room-a", "never-print-this");
        let debug = format!("{command:?}");

        assert!(!debug.contains("never-print-this"));
        assert!(debug.contains("<redacted>"));
    }

    #[tokio::test]
    async fn disabled_plex_configuration_keeps_service_unowned() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let config = PlexClientConfig {
            enabled: false,
            ..PlexClientConfig::default()
        };

        let events = application
            .configure_plex_service(&config, "test-client", None)
            .await;

        assert!(events.is_empty());
        assert!(!application.plex_service_enabled());
    }
}
