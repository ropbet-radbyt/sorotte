use serde_json::Value;
pub use sorotte_client_core::ConnectionPhase;
use sorotte_client_core::{
    AutoplayCountdownNotification, ChatNotification, ClientEffect, ClientEffectError,
    ClientPlayerIo, ClientRuntime, ClientSession, ClientSessionUpdate,
    ControlledRoomCreationNotification, ControllerAuthTransitionNotification, CoordinatorCommandId,
    FileSize, LogicalMediaId, MediaLoadIntent, MediaLoadPlan, MediaTransportKind,
    PendingProtocolLine, PlaybackBarrierRoomBufferingConfig, PlaybackBarrierStartConfig,
    PlaybackBarrierTimeoutAction, PlaybackCoordinationSnapshot, PlaybackCoordinatorAction,
    PlaybackCoordinatorConfig, PrivacyMode, ProtocolLineLease, QueuedRuntimeControl,
    ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionStateSnapshot,
    ReconnectTransitionNotification, RoomPlaystateView, UserChangeNotification,
};
use sorotte_player_api::{
    PlayerAdapter, PlayerCommandId, PlayerError, PlayerPlaybackTelemetryUpdate,
    PlayerTransportTelemetryUpdate,
};
pub use sorotte_plex::PlexClientConfig;
use sorotte_plex::{
    cache::PlexMatchCache,
    http::PlexHttpClient,
    timeline::{PlexSyncEngine, PlexWatchEvent},
};
use sorotte_protocol::{
    DirectReadinessSurface, ProtocolError, ProtocolMessage, StatePayload, decode_message_line_items,
};
use sorotte_secret::SecretValue;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::{
    legacy_settings::AutoplayThresholdOverride,
    runtime_config::{
        ClientConfig, RoomBufferingPolicy, StartSynchronizationPolicy, StartTimeoutAction,
        StreamingPlaybackConfig, StreamingQualityDowngradeSuggestion,
    },
};

const PLEX_SYNC_PUMP_INTERVAL: Duration = Duration::from_secs(1);
const PLAYER_INTEGRATION_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(100);

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

#[derive(Clone, PartialEq, Default)]
pub struct ClientApplicationSettings {
    pub config: ClientConfig,
    pub active_room: Option<String>,
}

impl ClientApplicationSettings {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            active_room: None,
        }
    }

    pub fn with_active_room(mut self, active_room: impl Into<String>) -> Self {
        self.active_room = Some(active_room.into());
        self
    }
}

impl std::fmt::Debug for ClientApplicationSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientApplicationSettings")
            .field("config", &self.config)
            .field("has_active_room", &self.active_room.is_some())
            .finish()
    }
}

/// Result of applying the decodable prefix of one transport line.
///
/// A command-level decode error is retained separately so adapters can publish
/// effects produced by earlier commands before they surface the error. Errors
/// while parsing the line itself or applying a decoded command are still
/// returned immediately.
pub struct ProtocolLineApplyOutcome {
    pub state_sync_emitted: bool,
    pub applied_message_count: usize,
    pub trailing_decode_error: Option<ProtocolError>,
}

#[derive(Clone, PartialEq)]
pub enum ClientCommand {
    Connect {
        endpoint: String,
    },
    BeginConnecting,
    InitializeSessionIdentity {
        username: String,
        room: String,
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
    SetReadyFrom {
        username: Option<String>,
        ready: Option<bool>,
        manually_initiated: bool,
        surface: DirectReadinessSurface,
    },
    OpenMedia {
        path: String,
    },
    PlayerPlaybackObserved(PlayerPlaybackTelemetryUpdate),
    UpdateSettings(Box<ClientApplicationSettings>),
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

impl std::fmt::Debug for ClientCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReceiveProtocolLine {
                line,
                received_at_seconds,
            } => formatter
                .debug_struct("ReceiveProtocolLine")
                .field("line_bytes", &line.len())
                .field("received_at_seconds", received_at_seconds)
                .finish(),
            Self::OpenMedia { .. } => formatter
                .debug_struct("OpenMedia")
                .field("path", &sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::RequestControllerAuth { room, password } => formatter
                .debug_struct("RequestControllerAuth")
                .field("room", room)
                .field("password", password)
                .finish(),
            Self::Connect { .. } => formatter.write_str("Connect"),
            Self::BeginConnecting => formatter.write_str("BeginConnecting"),
            Self::InitializeSessionIdentity { .. } => {
                formatter.write_str("InitializeSessionIdentity")
            }
            Self::TransportConnected => formatter.write_str("TransportConnected"),
            Self::Reconnect { .. } => formatter.write_str("Reconnect"),
            Self::Disconnect { .. } => formatter.write_str("Disconnect"),
            Self::SetRoom { .. } => formatter.write_str("SetRoom"),
            Self::SetReady { .. } => formatter.write_str("SetReady"),
            Self::SetReadyFrom { .. } => formatter.write_str("SetReadyFrom"),
            Self::PlayerPlaybackObserved(_) => formatter.write_str("PlayerPlaybackObserved"),
            Self::UpdateSettings(_) => formatter.write_str("UpdateSettings"),
            Self::SendChat(_) => formatter.write_str("SendChat"),
            Self::RequestUserList => formatter.write_str("RequestUserList"),
            Self::SetPlaylistIndex(_) => formatter.write_str("SetPlaylistIndex"),
            Self::AdvancePlaylistIndex => formatter.write_str("AdvancePlaylistIndex"),
            Self::QueuePlaylistItem { .. } => formatter.write_str("QueuePlaylistItem"),
            Self::DeletePlaylistIndex(_) => formatter.write_str("DeletePlaylistIndex"),
            Self::UndoPlaylistChange => formatter.write_str("UndoPlaylistChange"),
            Self::ShuffleRemainingPlaylist => formatter.write_str("ShuffleRemainingPlaylist"),
            Self::ShuffleEntirePlaylist => formatter.write_str("ShuffleEntirePlaylist"),
            Self::UndoSeek => formatter.write_str("UndoSeek"),
            Self::SeekToPosition(_) => formatter.write_str("SeekToPosition"),
            Self::SeekByOffset(_) => formatter.write_str("SeekByOffset"),
            Self::TogglePause => formatter.write_str("TogglePause"),
            Self::SetPaused(_) => formatter.write_str("SetPaused"),
        }
    }
}

impl ClientCommand {
    pub fn update_settings(settings: ClientApplicationSettings) -> Self {
        Self::UpdateSettings(Box::new(settings))
    }

    pub fn request_controller_auth(
        room: impl Into<String>,
        password: impl Into<SecretValue>,
    ) -> Self {
        Self::RequestControllerAuth {
            room: room.into(),
            password: password.into(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum ClientEvent {
    ConnectionChanged(ConnectionPhase),
    RoomChanged {
        previous: Option<String>,
        current: Option<String>,
    },
    PlaybackChanged {
        paused: Option<bool>,
        position_seconds: Option<f64>,
        playback_rate: Option<f64>,
        paused_for_cache: Option<bool>,
        cache_buffering_percent: Option<f64>,
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

impl std::fmt::Debug for ClientEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionChanged(value) => formatter
                .debug_tuple("ConnectionChanged")
                .field(value)
                .finish(),
            Self::RoomChanged { previous, current } => formatter
                .debug_struct("RoomChanged")
                .field("previous", previous)
                .field("current", current)
                .finish(),
            Self::PlaybackChanged {
                paused,
                position_seconds,
                playback_rate,
                paused_for_cache,
                cache_buffering_percent,
            } => formatter
                .debug_struct("PlaybackChanged")
                .field("paused", paused)
                .field("position_seconds", position_seconds)
                .field("playback_rate", playback_rate)
                .field("paused_for_cache", paused_for_cache)
                .field("cache_buffering_percent", cache_buffering_percent)
                .finish(),
            Self::ReadinessChanged { username, ready } => formatter
                .debug_struct("ReadinessChanged")
                .field("username", username)
                .field("ready", ready)
                .finish(),
            Self::CommandCompleted { command, changed } => formatter
                .debug_struct("CommandCompleted")
                .field("command", command)
                .field("changed", changed)
                .finish(),
            Self::Notification(message) => formatter
                .debug_tuple("Notification")
                .field(message)
                .finish(),
            Self::OperationFailed { operation, .. } => formatter
                .debug_struct("OperationFailed")
                .field("operation", operation)
                .field("message", &sorotte_secret::REDACTED_SECRET)
                .finish(),
        }
    }
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
    playback_rate: Option<f64>,
    paused_for_cache: Option<bool>,
    cache_buffering_percent: Option<f64>,
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
    streaming_playback_config: StreamingPlaybackConfig,
}

impl<P> ClientApplication<P>
where
    P: PlayerAdapter,
{
    async fn await_worker_with_player_integration_maintenance<T>(
        &mut self,
        mut worker: tokio::task::JoinHandle<T>,
    ) -> Result<T, tokio::task::JoinError> {
        let mut tick = tokio::time::interval(PLAYER_INTEGRATION_MAINTENANCE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await;
        loop {
            tokio::select! {
                result = &mut worker => return result,
                _ = tick.tick() => {
                    self.runtime
                        .with_player_io(PlayerAdapter::maintain_runtime_integrations);
                }
            }
        }
    }

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
            streaming_playback_config: StreamingPlaybackConfig::default(),
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
        let worker = tokio::task::spawn_blocking(move || {
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
        });
        match self
            .await_worker_with_player_integration_maintenance(worker)
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
        self.runtime
            .with_player_io(PlayerAdapter::maintain_runtime_integrations);
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
            match self
                .await_worker_with_player_integration_maintenance(worker)
                .await
            {
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
        let worker = tokio::task::spawn_blocking(move || {
            let before = engine.cache().clone();
            let _ = engine.tick(None, SystemTime::now());
            plex_cache_save_error_if_changed(&engine, cache_path.as_deref(), &before)
        });
        match self
            .await_worker_with_player_integration_maintenance(worker)
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

    pub fn player_mut(&mut self) -> ClientPlayerIo<'_, P, QueuedRuntimeControl> {
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
            playback_rate: session.local_playback_rate(),
            paused_for_cache: session.local_paused_for_cache(),
            cache_buffering_percent: session.local_cache_buffering_percent(),
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
        if before.paused != after.paused
            || before.position_seconds != after.position_seconds
            || before.playback_rate != after.playback_rate
            || before.paused_for_cache != after.paused_for_cache
            || before.cache_buffering_percent != after.cache_buffering_percent
        {
            events.push(ClientEvent::PlaybackChanged {
                paused: after.paused,
                position_seconds: after.position_seconds,
                playback_rate: after.playback_rate,
                paused_for_cache: after.paused_for_cache,
                cache_buffering_percent: after.cache_buffering_percent,
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

    fn apply_settings(&mut self, settings: ClientApplicationSettings) {
        // Build complete replacement slices before borrowing the session mutably. The
        // validated application command is infallible, and no adapter or I/O callback
        // can observe a partially applied configuration.
        let (mut behavior, mut desync, mut readiness) = {
            let session = self.runtime.session();
            (
                session.behavior_config().clone(),
                session.desync_config().clone(),
                session.readiness_autoplay_config().clone(),
            )
        };
        let config = settings.config;
        self.streaming_playback_config = config.playback.streaming.clone();
        self.runtime.set_playback_coordinator_config(
            config.playback.streaming.playback_coordinator_config(),
        );
        let start = &config.playback.streaming.start_synchronization;
        self.runtime
            .set_playback_barrier_start_config(PlaybackBarrierStartConfig {
                policy: match start.policy {
                    StartSynchronizationPolicy::Immediate => None,
                    StartSynchronizationPolicy::WaitForController => {
                        Some(sorotte_protocol::PlaybackBarrierPolicy::Controller)
                    }
                    StartSynchronizationPolicy::WaitForAllEligible => {
                        Some(sorotte_protocol::PlaybackBarrierPolicy::AllEligible)
                    }
                    StartSynchronizationPolicy::Quorum => {
                        Some(sorotte_protocol::PlaybackBarrierPolicy::Quorum)
                    }
                },
                quorum_percent: start.quorum.get().round() as u32,
                timeout_seconds: start.timeout.get(),
                timeout_action: match start.timeout_action {
                    StartTimeoutAction::Continue => PlaybackBarrierTimeoutAction::Continue,
                    StartTimeoutAction::RemainPaused => PlaybackBarrierTimeoutAction::RemainPaused,
                    StartTimeoutAction::AskController => {
                        PlaybackBarrierTimeoutAction::AskController
                    }
                },
            });
        let room_buffering = &config.playback.streaming.room_buffering;
        self.runtime.set_playback_barrier_room_buffering_config(
            PlaybackBarrierRoomBufferingConfig {
                policy: match room_buffering.policy {
                    RoomBufferingPolicy::Independent => {
                        sorotte_protocol::RoomBufferingPolicy::Independent
                    }
                    RoomBufferingPolicy::PauseController => {
                        sorotte_protocol::RoomBufferingPolicy::PauseController
                    }
                    RoomBufferingPolicy::PauseEligible => {
                        sorotte_protocol::RoomBufferingPolicy::PauseAnyEligible
                    }
                    RoomBufferingPolicy::Quorum => sorotte_protocol::RoomBufferingPolicy::Quorum,
                },
                quorum_percent: room_buffering.quorum.get().round() as u32,
                maximum_pause_seconds: room_buffering.maximum_pause.get(),
                ..PlaybackBarrierRoomBufferingConfig::default()
            },
        );

        behavior.show_same_room_osd = config.interface.show_same_room_osd;
        behavior.show_osd_warnings = config.interface.show_osd_warnings;
        behavior.show_noncontroller_osd = config.interface.show_noncontroller_osd;
        behavior.show_different_room_osd = config.interface.show_different_room_osd;
        behavior.pause_on_leave = config.playback.pause_on_leave;
        behavior.loop_at_end_of_playlist = config.playback.loop_at_end_of_playlist;
        behavior.loop_single_files = config.playback.loop_single_files;
        behavior.only_switch_to_trusted_domains = config.playback.only_switch_to_trusted_domains;
        behavior.trusted_domains = config.playback.trusted_domains.clone();

        desync.rewind_on_desync = config.synchronization.rewind_on_desync;
        desync.fastforward_on_desync = config.synchronization.fastforward_on_desync;
        desync.slow_on_desync = config.synchronization.slow_on_desync;
        desync.rewind_threshold_seconds = config.synchronization.rewind_threshold.get();
        desync.fastforward_threshold_seconds = config.synchronization.fastforward_threshold.get();
        desync.slowdown_threshold_seconds = config.synchronization.slowdown_threshold.get();

        readiness.autoplay_require_same_filenames =
            config.readiness.autoplay_require_same_filenames;
        readiness.unpause_action = config.readiness.unpause_action.clone();
        readiness.auto_play_threshold = match config.readiness.autoplay_min_users {
            AutoplayThresholdOverride::Disable => None,
            AutoplayThresholdOverride::Set(value) => Some(value),
        };
        readiness.show_duration_notification = config.readiness.show_duration_notification;

        let autoplay_enabled = config.readiness.autoplay_initial_state;
        let controlled_room_password = config.connection.controlled_room_password;
        let mut session = self.runtime.session_mut();
        session.set_behavior_config(behavior);
        session.set_desync_config(desync);
        session.set_readiness_autoplay_config(readiness);
        session.set_autoplay_enabled(autoplay_enabled);
        if let (Some(room), Some(password)) = (settings.active_room, controlled_room_password) {
            // An absent configured password intentionally does not erase credentials
            // learned during this live session. A fresh application/session, including
            // the GUI's reset reconnect path, naturally starts with an empty cache.
            session.remember_control_password_for_room(&room, password);
        }
    }

    pub fn dispatch(&mut self, command: ClientCommand) -> Vec<ClientEvent> {
        let before = self.snapshot();
        let (operation, result) = match command {
            ClientCommand::Connect { endpoint } => {
                self.endpoint = Some(endpoint);
                self.runtime.begin_protocol_connection_generation();
                return self.set_connection_phase(ConnectionPhase::Connecting);
            }
            ClientCommand::BeginConnecting => {
                return match self.connection_phase() {
                    ConnectionPhase::Disconnected
                    | ConnectionPhase::AwaitingHello
                    | ConnectionPhase::Reconnecting { .. } => {
                        self.runtime.begin_protocol_connection_generation();
                        self.set_connection_phase(ConnectionPhase::Connecting)
                    }
                    ConnectionPhase::Connecting => Vec::new(),
                    phase => vec![ClientEvent::OperationFailed {
                        operation: "begin-connecting",
                        message: format!(
                            "a connection attempt cannot begin while the session is {phase:?}"
                        ),
                    }],
                };
            }
            ClientCommand::TransportConnected => {
                return match self.connection_phase() {
                    ConnectionPhase::Connecting | ConnectionPhase::Reconnecting { .. } => {
                        self.set_connection_phase(ConnectionPhase::AwaitingHello)
                    }
                    ConnectionPhase::AwaitingHello => Vec::new(),
                    phase => vec![ClientEvent::OperationFailed {
                        operation: "transport-connected",
                        message: format!(
                            "transport connection cannot complete while the session is {phase:?}"
                        ),
                    }],
                };
            }
            ClientCommand::Reconnect { attempt } => {
                self.runtime.begin_protocol_connection_generation();
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
            ClientCommand::InitializeSessionIdentity { username, room } => {
                if matches!(
                    self.connection_phase(),
                    ConnectionPhase::Connecting
                        | ConnectionPhase::AwaitingHello
                        | ConnectionPhase::Reconnecting { .. }
                ) {
                    self.runtime
                        .session_mut()
                        .initialize_local_identity(username, room);
                    ("initialize-session-identity", Ok(true))
                } else {
                    (
                        "initialize-session-identity",
                        Err(PlayerError::OperationFailed(format!(
                            "session identity cannot be initialized while the session is {:?}",
                            self.connection_phase()
                        ))),
                    )
                }
            }
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
            ClientCommand::SetReadyFrom {
                username,
                ready,
                manually_initiated,
                surface,
            } => (
                "set-ready",
                match (username, ready) {
                    (Some(username), Some(ready)) => self.runtime.run_set_ready_for_user_from(
                        username,
                        ready,
                        manually_initiated,
                        surface,
                    ),
                    (None, Some(ready)) => self.runtime.run_set_ready_for_user_from(
                        "",
                        ready,
                        manually_initiated,
                        surface,
                    ),
                    (_, None) => self
                        .runtime
                        .run_toggle_ready_from(manually_initiated, surface),
                },
            ),
            ClientCommand::OpenMedia { path } => (
                "open-media",
                self.runtime.player_mut().open_file(&path).map(|()| true),
            ),
            ClientCommand::PlayerPlaybackObserved(update) => {
                let changed = self
                    .runtime
                    .session_mut()
                    .apply_player_playback_telemetry_update(&update);
                ("player-playback-observed", Ok(changed))
            }
            ClientCommand::UpdateSettings(settings) => {
                self.apply_settings(*settings);
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
                self.runtime.run_request_controller_auth(room, password),
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
            Err(error) => {
                let mut events = self.domain_events_since(&before);
                events.push(ClientEvent::OperationFailed {
                    operation,
                    message: error.to_string(),
                });
                events
            }
        }
    }

    pub fn pending_protocol_line(&self) -> Result<Option<PendingProtocolLine>, ProtocolError> {
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
        let outcome = self.apply_protocol_line_prefix(
            line,
            received_at_seconds,
            reconcile_inbound_state,
            dont_slow_down_with_me,
            apply_fallback_json,
        )?;
        if let Some(error) = outcome.trailing_decode_error {
            return Err(error);
        }
        Ok(outcome.state_sync_emitted)
    }

    /// Applies commands in wire order until the first command-level decode
    /// error, returning that error alongside the effects of the valid prefix.
    pub fn apply_protocol_line_prefix(
        &mut self,
        line: &str,
        received_at_seconds: f64,
        reconcile_inbound_state: bool,
        dont_slow_down_with_me: bool,
        apply_fallback_json: bool,
    ) -> Result<ProtocolLineApplyOutcome, ProtocolError> {
        let messages = decode_message_line_items(line)?;
        if messages.is_empty() {
            if apply_fallback_json {
                self.runtime
                    .session_mut()
                    .apply_message_json_at(line, received_at_seconds)?;
            }
            return Ok(ProtocolLineApplyOutcome {
                state_sync_emitted: false,
                applied_message_count: 0,
                trailing_decode_error: None,
            });
        }

        let mut state_sync_emitted = false;
        let mut applied_message_count = 0;
        for item in messages {
            let message = match item.message {
                Ok(message) => message,
                Err(error) => {
                    return Ok(ProtocolLineApplyOutcome {
                        state_sync_emitted,
                        applied_message_count,
                        trailing_decode_error: Some(error),
                    });
                }
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
            applied_message_count += 1;
        }
        Ok(ProtocolLineApplyOutcome {
            state_sync_emitted,
            applied_message_count,
            trailing_decode_error: None,
        })
    }

    pub fn acknowledge_protocol_line(
        &mut self,
        lease: ProtocolLineLease,
    ) -> Option<ProtocolMessage> {
        self.runtime.acknowledge_protocol_line(lease)
    }

    pub fn release_protocol_line(&mut self, lease: ProtocolLineLease) -> bool {
        self.runtime.release_protocol_line(lease)
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

    pub fn run_initial_readiness_intent(&mut self, ready: bool) -> Result<bool, PlayerError> {
        self.runtime.run_initial_readiness_intent(ready)
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
        password: impl Into<SecretValue>,
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

    /// Emits a correlated playback-coordination retry once its server-provided
    /// backoff has elapsed. Repeated calls are safe and emit at most one
    /// attempt for the current operation.
    pub fn run_pending_playback_barrier_retry_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .run_pending_playback_barrier_retry_at(now_seconds)
    }

    /// Returns the remaining delay before a pending playback-coordination
    /// attempt may be retried.
    pub fn pending_playback_barrier_retry_delay_at(&self, now_seconds: f64) -> Option<f64> {
        self.runtime
            .pending_playback_barrier_retry_delay_at(now_seconds)
    }

    pub fn run_room_pause_sync_if_needed(&mut self) -> Result<(), PlayerError> {
        self.runtime.run_room_pause_sync_if_needed()
    }

    pub fn run_room_pause_sync_if_needed_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime.run_room_pause_sync_if_needed_at(now_seconds)
    }

    pub fn set_playback_coordinator_config(&mut self, config: PlaybackCoordinatorConfig) {
        self.runtime.set_playback_coordinator_config(config);
    }

    pub fn set_playback_barrier_start_config(&mut self, config: PlaybackBarrierStartConfig) {
        self.runtime.set_playback_barrier_start_config(config);
    }

    pub fn set_playback_barrier_room_buffering_config(
        &mut self,
        config: PlaybackBarrierRoomBufferingConfig,
    ) {
        self.runtime
            .set_playback_barrier_room_buffering_config(config);
    }

    pub fn prepare_playback_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        self.runtime
            .prepare_playback_media(logical_id, kind, now_seconds)
    }

    pub fn prepare_playback_media_with_intent(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> MediaLoadPlan {
        self.runtime
            .prepare_playback_media_with_intent(logical_id, kind, intent, now_seconds)
    }

    pub fn observe_external_player_transport(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .observe_external_player_transport(update, now_seconds)
    }

    pub fn observe_external_player_transport_at_epoch(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
        adapter_epoch: u64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .observe_external_player_transport_at_epoch(update, now_seconds, adapter_epoch)
    }

    pub fn reconcile_external_player_playback(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime.reconcile_external_player_playback(now_seconds)
    }

    pub fn stage_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .stage_external_player_pause_intent(paused, now_seconds)
    }

    pub fn rollback_external_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .rollback_external_player_pause_intent(paused, now_seconds)
    }

    pub fn interrupt_external_playback_recovery(&mut self) -> Vec<PlaybackCoordinatorAction> {
        self.runtime.interrupt_external_playback_recovery()
    }

    pub fn run_keep_waiting_for_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .run_keep_waiting_for_seek_preparation(now_seconds)
    }

    pub fn run_cancel_seek_preparation(&mut self, now_seconds: f64) -> Result<bool, PlayerError> {
        self.runtime.run_cancel_seek_preparation(now_seconds)
    }

    pub fn run_join_nearest_buffered_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .run_join_nearest_buffered_seek_preparation(now_seconds)
    }

    pub fn keep_waiting_for_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .keep_waiting_for_external_seek_preparation(now_seconds)
    }

    pub fn cancel_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime.cancel_external_seek_preparation(now_seconds)
    }

    pub fn join_nearest_buffered_external_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Vec<PlaybackCoordinatorAction> {
        self.runtime
            .join_nearest_buffered_external_seek_preparation(now_seconds)
    }

    pub fn report_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        result: Result<(), PlayerError>,
        now_seconds: f64,
    ) {
        self.runtime
            .report_external_coordinator_command_dispatch(command_id, result, now_seconds);
    }

    pub fn begin_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        self.runtime
            .begin_external_coordinator_command_dispatch(command_id, now_seconds)
    }

    pub fn finish_external_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        player_command_id: Option<PlayerCommandId>,
        result: Result<(), PlayerError>,
        now_seconds: f64,
    ) {
        self.runtime.finish_external_coordinator_command_dispatch(
            command_id,
            player_command_id,
            result,
            now_seconds,
        );
    }

    pub fn playback_coordination_snapshot(&self) -> PlaybackCoordinationSnapshot {
        self.runtime.playback_coordination_snapshot()
    }

    pub fn streaming_quality_downgrade_suggestion(
        &self,
        approximate_selected_bitrate_bytes_per_second: Option<u64>,
    ) -> Option<StreamingQualityDowngradeSuggestion> {
        self.streaming_playback_config.quality_downgrade_suggestion(
            &self.runtime.playback_coordination_snapshot().metrics,
            approximate_selected_bitrate_bytes_per_second,
        )
    }

    pub fn take_playback_barrier_timeout_action(&mut self) -> Option<PlaybackBarrierTimeoutAction> {
        self.runtime.take_playback_barrier_timeout_action()
    }

    pub fn reset_playback_transport_adapter_epoch(&mut self, now_seconds: f64) -> u64 {
        self.runtime
            .reset_playback_transport_adapter_epoch(now_seconds)
    }

    pub fn playback_transport_adapter_epoch(&self) -> u64 {
        self.runtime.playback_transport_adapter_epoch()
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

    pub fn run_direct_player_readiness_intent(
        &mut self,
        paused: bool,
        surface: sorotte_protocol::PlayerInteractionSurface,
    ) -> Result<bool, PlayerError> {
        self.runtime
            .run_direct_player_readiness_intent(paused, surface)
    }

    pub fn confirm_pending_native_player_play(
        &mut self,
        surface: sorotte_protocol::PlayerInteractionSurface,
    ) -> Result<bool, PlayerError> {
        self.runtime.confirm_pending_native_player_play(surface)
    }

    pub fn readiness_gate_holds_current_playback(&self) -> bool {
        self.runtime.readiness_gate_holds_current_playback()
    }

    pub fn record_external_player_pause_command_result(
        &mut self,
        paused: bool,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .record_external_player_pause_command_result(paused, succeeded, now_seconds)
    }

    pub fn begin_external_player_pause_command(
        &mut self,
        paused: bool,
        cause: sorotte_client_core::PlayerCommandCause,
        now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        self.runtime
            .begin_external_player_pause_command(paused, cause, now_seconds)
    }

    pub fn finish_external_player_pause_command(
        &mut self,
        command_id: Option<PlayerCommandId>,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .finish_external_player_pause_command(command_id, succeeded, now_seconds)
    }

    pub fn record_external_system_player_pause_command_result(
        &mut self,
        paused: bool,
        cause: sorotte_client_core::PlayerCommandCause,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .record_external_system_player_pause_command_result(
                paused,
                cause,
                succeeded,
                now_seconds,
            )
    }

    pub fn observe_external_player_end_of_file(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .observe_external_player_end_of_file(now_seconds)
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

    pub fn run_controller_auth_notifications_if_needed_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), PlayerError> {
        self.runtime
            .run_controller_auth_notifications_if_needed_at(now_seconds)
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct TestPlayer {
        opened: Vec<String>,
        paused: bool,
        open_error: Option<String>,
        pause_error: Option<String>,
        maintenance_calls: Arc<AtomicUsize>,
    }

    impl PlayerAdapter for TestPlayer {
        fn name(&self) -> &'static str {
            "client-application-test"
        }

        fn maintain_runtime_integrations(&mut self) {
            self.maintenance_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
            self.opened.push(path.to_owned());
            match self.open_error.as_ref() {
                Some(message) => Err(PlayerError::OperationFailed(message.clone())),
                None => Ok(()),
            }
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
            self.paused = paused;
            match self.pause_error.as_ref() {
                Some(message) => Err(PlayerError::OperationFailed(message.clone())),
                None => Ok(()),
            }
        }
    }

    fn configured_runtime_settings() -> ClientConfig {
        let mut config = ClientConfig::default();
        config.interface.show_same_room_osd = false;
        config.interface.show_osd_warnings = false;
        config.interface.show_noncontroller_osd = true;
        config.interface.show_different_room_osd = true;
        config.playback.pause_on_leave = false;
        config.playback.loop_at_end_of_playlist = true;
        config.playback.loop_single_files = true;
        config.playback.only_switch_to_trusted_domains = false;
        config.playback.trusted_domains = vec!["*.example.test/media".to_owned()];
        config.synchronization.rewind_on_desync = false;
        config.synchronization.fastforward_on_desync = false;
        config.synchronization.slow_on_desync = false;
        config.synchronization.rewind_threshold =
            crate::runtime_config::Seconds::new(1.25).expect("valid rewind threshold");
        config.synchronization.fastforward_threshold =
            crate::runtime_config::Seconds::new(4.5).expect("valid fast-forward threshold");
        config.synchronization.slowdown_threshold =
            crate::runtime_config::Seconds::new(0.75).expect("valid slowdown threshold");
        config.readiness.autoplay_initial_state = true;
        config.readiness.autoplay_require_same_filenames = true;
        config.readiness.unpause_action = sorotte_client_core::UnpauseActionMode::Always;
        config.readiness.autoplay_min_users = AutoplayThresholdOverride::Set(4);
        config.readiness.show_duration_notification = false;
        config
    }

    fn assert_runtime_settings_match(
        application: &ClientApplication<TestPlayer>,
        config: &ClientConfig,
    ) {
        let session = application.session();
        let behavior = session.behavior_config();
        assert_eq!(
            behavior.show_same_room_osd,
            config.interface.show_same_room_osd
        );
        assert_eq!(
            behavior.show_osd_warnings,
            config.interface.show_osd_warnings
        );
        assert_eq!(
            behavior.show_noncontroller_osd,
            config.interface.show_noncontroller_osd
        );
        assert_eq!(
            behavior.show_different_room_osd,
            config.interface.show_different_room_osd
        );
        assert_eq!(behavior.pause_on_leave, config.playback.pause_on_leave);
        assert_eq!(
            behavior.loop_at_end_of_playlist,
            config.playback.loop_at_end_of_playlist
        );
        assert_eq!(
            behavior.loop_single_files,
            config.playback.loop_single_files
        );
        assert_eq!(
            behavior.only_switch_to_trusted_domains,
            config.playback.only_switch_to_trusted_domains
        );
        assert_eq!(behavior.trusted_domains, config.playback.trusted_domains);

        let desync = session.desync_config();
        assert_eq!(
            desync.rewind_on_desync,
            config.synchronization.rewind_on_desync
        );
        assert_eq!(
            desync.fastforward_on_desync,
            config.synchronization.fastforward_on_desync
        );
        assert_eq!(desync.slow_on_desync, config.synchronization.slow_on_desync);
        assert_eq!(
            desync.rewind_threshold_seconds,
            config.synchronization.rewind_threshold.get()
        );
        assert_eq!(
            desync.fastforward_threshold_seconds,
            config.synchronization.fastforward_threshold.get()
        );
        assert_eq!(
            desync.slowdown_threshold_seconds,
            config.synchronization.slowdown_threshold.get()
        );

        let readiness = session.readiness_autoplay_config();
        assert_eq!(
            readiness.autoplay_require_same_filenames,
            config.readiness.autoplay_require_same_filenames
        );
        assert_eq!(readiness.unpause_action, config.readiness.unpause_action);
        assert_eq!(
            readiness.auto_play_threshold,
            match config.readiness.autoplay_min_users {
                AutoplayThresholdOverride::Disable => None,
                AutoplayThresholdOverride::Set(value) => Some(value),
            }
        );
        assert_eq!(
            readiness.show_duration_notification,
            config.readiness.show_duration_notification
        );
        assert_eq!(
            session.autoplay_enabled(),
            config.readiness.autoplay_initial_state
        );
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
        let identity_events = application.dispatch(ClientCommand::InitializeSessionIdentity {
            username: "alice".to_owned(),
            room: "room-a".to_owned(),
        });
        assert!(identity_events.iter().any(|event| matches!(
            event,
            ClientEvent::RoomChanged { current, .. } if current.as_deref() == Some("room-a")
        )));
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Connecting
        ));
        assert_eq!(
            application.dispatch(ClientCommand::TransportConnected),
            vec![ClientEvent::ConnectionChanged(
                ConnectionPhase::AwaitingHello,
            )],
        );
        assert_eq!(
            application.dispatch(ClientCommand::Reconnect { attempt: 2 }),
            vec![ClientEvent::ConnectionChanged(
                ConnectionPhase::Reconnecting { attempt: 2 },
            )],
        );
        assert_eq!(
            application.dispatch(ClientCommand::TransportConnected),
            vec![ClientEvent::ConnectionChanged(
                ConnectionPhase::AwaitingHello,
            )],
        );
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::AwaitingHello
        ));

        let events = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5","features":{"chat":true}}}"#
                .to_owned(),
            received_at_seconds: 1.0,
        });
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::ConnectionChanged(ConnectionPhase::Active(capabilities))
                if capabilities.chat
        )));

        let identity_events = application.dispatch(ClientCommand::InitializeSessionIdentity {
            username: "mallory".to_owned(),
            room: "wrong-room".to_owned(),
        });
        assert!(identity_events.iter().any(|event| matches!(
            event,
            ClientEvent::OperationFailed {
                operation: "initialize-session-identity",
                ..
            }
        )));
        assert_eq!(application.session().username(), Some("alice"));
        assert_eq!(application.session().room(), Some("room-a"));

        let transport_events = application.dispatch(ClientCommand::TransportConnected);
        assert!(transport_events.iter().any(|event| matches!(
            event,
            ClientEvent::OperationFailed {
                operation: "transport-connected",
                ..
            }
        )));
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Active(_)
        ));
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
        let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(12.5),
        ));

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
    fn application_forwards_seek_preparation_actions_without_side_effects_when_inactive() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        assert!(
            application
                .keep_waiting_for_external_seek_preparation(1.0)
                .is_empty()
        );
        assert!(
            application
                .join_nearest_buffered_external_seek_preparation(1.0)
                .is_empty()
        );
        assert!(application.cancel_external_seek_preparation(1.0).is_empty());
        assert!(
            !application
                .run_keep_waiting_for_seek_preparation(1.0)
                .expect("inactive keep-waiting should remain a valid no-op")
        );
        assert!(
            !application
                .run_join_nearest_buffered_seek_preparation(1.0)
                .expect("inactive nearest-buffered join should remain a valid no-op")
        );
        assert!(
            !application
                .run_cancel_seek_preparation(1.0)
                .expect("inactive cancellation should remain a valid no-op")
        );
        assert!(
            application
                .playback_coordination_snapshot()
                .seek_preparation
                .is_none()
        );
    }

    #[test]
    fn application_applies_complete_player_playback_telemetry_updates() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        let events = application.dispatch(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(false)
                .with_position_seconds(42.5)
                .with_playback_rate(0.95)
                .with_paused_for_cache(false)
                .with_cache_buffering_percent(37.5),
        ));

        assert_eq!(application.session().local_paused(), Some(false));
        assert_eq!(application.session().local_position_seconds(), Some(42.5));
        assert_eq!(application.session().local_playback_rate(), Some(0.95));
        assert_eq!(application.session().local_paused_for_cache(), Some(false));
        assert_eq!(
            application.session().local_cache_buffering_percent(),
            Some(37.5)
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::PlaybackChanged {
                paused: Some(false),
                position_seconds: Some(42.5),
                playback_rate: Some(0.95),
                paused_for_cache: Some(false),
                cache_buffering_percent: Some(37.5),
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::CommandCompleted {
                command: "player-playback-observed",
                changed: true,
            }
        )));

        let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate::default().with_playback_rate(f64::NAN),
        ));
        assert_eq!(application.session().local_playback_rate(), Some(0.95));
        application.session_mut().reset_sync_state_for_reconnect();
        assert_eq!(application.session().local_playback_rate(), None);
    }

    #[test]
    fn application_reports_unchanged_for_empty_identical_and_invalid_player_telemetry() {
        fn assert_unchanged(events: &[ClientEvent]) {
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, ClientEvent::PlaybackChanged { .. }))
            );
            assert!(events.iter().any(|event| matches!(
                event,
                ClientEvent::CommandCompleted {
                    command: "player-playback-observed",
                    changed: false,
                }
            )));
        }

        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        assert_unchanged(&application.dispatch(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate::default(),
        )));

        let accepted = PlayerPlaybackTelemetryUpdate::default()
            .with_paused(false)
            .with_position_seconds(42.5)
            .with_playback_rate(0.95)
            .with_paused_for_cache(false)
            .with_cache_buffering_percent(37.5);
        let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(accepted.clone()));
        assert_unchanged(&application.dispatch(ClientCommand::PlayerPlaybackObserved(accepted)));

        for invalid_update in [
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(f64::NAN)
                .with_playback_rate(f64::INFINITY)
                .with_cache_buffering_percent(f64::NEG_INFINITY),
            PlayerPlaybackTelemetryUpdate::default().with_playback_rate(-1.0),
            PlayerPlaybackTelemetryUpdate::default().with_playback_rate(0.0),
            PlayerPlaybackTelemetryUpdate::default().with_cache_buffering_percent(-0.1),
            PlayerPlaybackTelemetryUpdate::default().with_cache_buffering_percent(100.1),
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(-0.1),
        ] {
            assert_unchanged(
                &application.dispatch(ClientCommand::PlayerPlaybackObserved(invalid_update)),
            );
        }

        assert_eq!(application.session().local_paused(), Some(false));
        assert_eq!(application.session().local_position_seconds(), Some(42.5));
        assert_eq!(application.session().local_playback_rate(), Some(0.95));
        assert_eq!(application.session().local_paused_for_cache(), Some(false));
        assert_eq!(
            application.session().local_cache_buffering_percent(),
            Some(37.5)
        );
    }

    #[test]
    fn application_failure_preserves_final_domain_truth_before_operation_failed() {
        let player = TestPlayer {
            pause_error: Some("pause transport failed".to_owned()),
            ..TestPlayer::default()
        };
        let mut application = ClientApplication::new(ClientSession::default(), player);
        let mut behavior = application.session().behavior_config().clone();
        behavior.pause_on_leave = true;
        application.session_mut().set_behavior_config(behavior);
        let _ = application.dispatch(ClientCommand::Connect {
            endpoint: "sync.example:8999".to_owned(),
        });
        let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate::default().with_paused(false),
        ));

        let events = application.dispatch(ClientCommand::Disconnect { now_seconds: 1.0 });

        let connection_event = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    ClientEvent::ConnectionChanged(ConnectionPhase::Disconnected)
                )
            })
            .expect("disconnect state change should survive the player failure");
        let failure_event = events
            .iter()
            .position(|event| matches!(event, ClientEvent::OperationFailed { .. }))
            .expect("the player failure should still be surfaced");
        assert!(connection_event < failure_event);
        assert!(matches!(
            events.last(),
            Some(ClientEvent::OperationFailed { .. })
        ));
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Disconnected
        ));
    }

    #[test]
    fn application_settings_command_applies_full_runtime_slices_atomically() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        {
            let mut behavior = application.session().behavior_config().clone();
            let mut desync = application.session().desync_config().clone();
            let mut readiness = application.session().readiness_autoplay_config().clone();
            behavior.reconnect_state_restore_correction_retry_max_attempts = 91;
            desync.slowdown_rate = 0.8125;
            readiness.autoplay_delay_seconds = 17.25;

            let mut session = application.session_mut();
            session.set_behavior_config(behavior);
            session.set_desync_config(desync);
            session.set_readiness_autoplay_config(readiness);
        }
        let config = configured_runtime_settings();

        let events = application.dispatch(ClientCommand::update_settings(
            ClientApplicationSettings::new(config.clone()).with_active_room("room-a"),
        ));

        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::CommandCompleted {
                command: "update-settings",
                changed: true,
            }
        )));
        assert_runtime_settings_match(&application, &config);
        assert_eq!(
            application
                .session()
                .behavior_config()
                .reconnect_state_restore_correction_retry_max_attempts,
            91,
            "the application must preserve behavior fields outside validated ClientConfig ownership"
        );
        assert_eq!(application.session().desync_config().slowdown_rate, 0.8125);
        assert_eq!(
            application
                .session()
                .readiness_autoplay_config()
                .autoplay_delay_seconds,
            17.25
        );
    }

    #[test]
    fn application_settings_command_clears_mapped_values_to_validated_defaults() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let configured = configured_runtime_settings();
        let defaults = ClientConfig::default();

        let _ = application.dispatch(ClientCommand::update_settings(
            ClientApplicationSettings::new(configured).with_active_room("room-a"),
        ));
        let _ = application.dispatch(ClientCommand::update_settings(
            ClientApplicationSettings::new(defaults.clone()).with_active_room("room-a"),
        ));

        assert_runtime_settings_match(&application, &defaults);
    }

    #[test]
    fn application_settings_redact_and_cache_controlled_room_password() {
        const SECRET: &str = "APP-CONFIG-PASSWORD-123";
        const ROOM: &str = "+room:ABCDEF123456";
        let mut config = ClientConfig::default();
        config.connection.controlled_room_password = Some(SECRET.into());
        let settings = ClientApplicationSettings::new(config).with_active_room(ROOM);
        let settings_debug = format!("{settings:?}");
        let command = ClientCommand::update_settings(settings);
        let command_debug = format!("{command:?}");
        assert!(!settings_debug.contains(SECRET));
        assert!(!command_debug.contains(SECRET));
        assert!(settings_debug.contains(sorotte_secret::REDACTED_SECRET));

        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let _ = application.dispatch(command);
        let _ = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: format!(
                r#"{{"Hello":{{"username":"alice","room":{{"name":"{ROOM}"}},"version":"1.3.0","features":{{"managedRooms":true}}}}}}"#
            ),
            received_at_seconds: 1.0,
        });
        application
            .run_controller_reidentify_if_needed()
            .expect("configured controller password should produce a re-identification effect");

        let line = application
            .pending_protocol_line()
            .expect("controller-auth message should encode")
            .expect("configured controller password should be cached for the active room");
        assert!(line.line().contains(SECRET));

        let _ = application.acknowledge_protocol_line(line.lease());
        let _ = application.dispatch(ClientCommand::update_settings(
            ClientApplicationSettings::new(ClientConfig::default()).with_active_room(ROOM),
        ));
        let _ = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: format!(
                r#"{{"Hello":{{"username":"alice","room":{{"name":"{ROOM}"}},"version":"1.3.0","features":{{"managedRooms":true}}}}}}"#
            ),
            received_at_seconds: 2.0,
        });
        application.run_controller_reidentify_if_needed().expect(
            "clearing configured settings must not erase a password learned by the live session",
        );
        assert!(
            application
                .pending_protocol_line()
                .expect("cached controller-auth message should encode")
                .expect("live session should retain its cached controller password")
                .line()
                .contains(SECRET)
        );
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
    fn application_reconnect_drops_staged_state_and_retains_reliable_commands_until_new_hello() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let _ = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#
                .to_owned(),
            received_at_seconds: 1.0,
        });
        assert!(
            application
                .runtime
                .run_state_sync_heartbeat_legacy_ping_compatible(false)
        );
        let staged_state = application
            .pending_protocol_line()
            .expect("State should encode")
            .expect("State should be staged");
        assert!(staged_state.line().contains("\"State\""));
        application
            .runtime
            .emit_effect(ClientEffect::SendChat("retain-chat".to_owned()))
            .expect("chat should queue");
        application
            .runtime
            .emit_effect(ClientEffect::SetPlaylist(vec!["episode.mkv".to_owned()]))
            .expect("playlist command should queue");

        let _ = application.dispatch(ClientCommand::Reconnect { attempt: 1 });
        assert!(
            application
                .pending_protocol_messages()
                .iter()
                .all(|message| !matches!(message, ProtocolMessage::State(_)))
        );
        assert_eq!(application.pending_protocol_message_count(), 2);

        let _ = application.dispatch(ClientCommand::TransportConnected);
        let _ = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#
                .to_owned(),
            received_at_seconds: 2.0,
        });
        assert!(
            application
                .pending_protocol_messages()
                .iter()
                .all(|message| !matches!(message, ProtocolMessage::State(_)))
        );
        assert!(matches!(
            &application.pending_protocol_messages()[0],
            ProtocolMessage::Chat(message)
                if message.chat == sorotte_protocol::ChatPayload::Text("retain-chat".to_owned())
        ));
        assert!(matches!(
            &application.pending_protocol_messages()[1],
            ProtocolMessage::Set(message) if message.set.playlist_change.is_some()
        ));
    }

    #[test]
    fn application_begin_connecting_after_disconnect_starts_fresh_protocol_generation() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let _ = application.dispatch(ClientCommand::ReceiveProtocolLine {
            line: r#"{"Hello":{"username":"alice","room":{"name":"room-a"},"version":"1.7.5"}}"#
                .to_owned(),
            received_at_seconds: 1.0,
        });
        assert!(
            application
                .runtime
                .run_state_sync_heartbeat_legacy_ping_compatible(false)
        );
        let staged_state = application
            .pending_protocol_line()
            .expect("State should encode")
            .expect("State should be staged");
        assert!(staged_state.line().contains("\"State\""));
        application
            .runtime
            .emit_effect(ClientEffect::SendChat("retain-chat".to_owned()))
            .expect("chat should queue");

        let _ = application.dispatch(ClientCommand::Disconnect { now_seconds: 2.0 });
        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Disconnected
        ));
        assert!(
            application
                .pending_protocol_messages()
                .iter()
                .any(|message| matches!(message, ProtocolMessage::State(_)))
        );

        let _ = application.dispatch(ClientCommand::BeginConnecting);

        assert!(matches!(
            application.connection_phase(),
            ConnectionPhase::Connecting
        ));
        assert_eq!(application.pending_protocol_message_count(), 1);
        assert!(matches!(
            application.pending_protocol_messages().front(),
            Some(ProtocolMessage::Chat(message))
                if message.chat
                    == sorotte_protocol::ChatPayload::Text("retain-chat".to_owned())
        ));
    }

    #[test]
    fn application_protocol_line_applies_valid_prefix_before_malformed_known_command() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        let error = application
            .apply_protocol_line(
                r#"{"Set":{"room":{"name":"prefix-room"}},"List":42}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect_err("malformed List command should be reported");

        assert!(matches!(error, ProtocolError::InvalidJson(_)));
        assert_eq!(application.session().room(), Some("prefix-room"));
    }

    #[test]
    fn application_protocol_line_applies_valid_prefix_before_unknown_command() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        let error = application
            .apply_protocol_line(
                r#"{"Set":{"room":{"name":"prefix-room"}},"FutureCommand":{"value":1}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect_err("unknown command should be reported");

        assert!(matches!(error, ProtocolError::InvalidJson(_)));
        assert_eq!(application.session().room(), Some("prefix-room"));
    }

    #[test]
    fn application_protocol_line_stops_when_first_command_is_invalid() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        let error = application
            .apply_protocol_line(
                r#"{"FutureCommand":{"value":1},"Set":{"room":{"name":"must-not-apply"}}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect_err("leading unknown command should be reported");

        assert!(matches!(error, ProtocolError::InvalidJson(_)));
        assert_eq!(application.session().room(), None);
    }

    #[test]
    fn application_protocol_line_applies_several_valid_commands_then_stops_at_error() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());

        let error = application
            .apply_protocol_line(
                r#"{"Set":{"room":{"name":"prefix-room"}},"List":{"listed-room":{}},"State":{"playstate":{"position":42.0,"paused":true,"doSeek":true,"setBy":"alice"}},"FutureCommand":{"value":1},"Hello":{"username":"must-not-apply","room":{"name":"trailing-room"},"version":"1.2.255"}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect_err("unknown command should stop the ordered application pass");

        assert!(matches!(error, ProtocolError::InvalidJson(_)));
        assert_eq!(application.session().room(), Some("prefix-room"));
        assert_eq!(
            application.session().room_names(),
            vec!["listed-room".to_owned(), "prefix-room".to_owned()]
        );
        let playstate = application
            .session()
            .current_room_playstate()
            .expect("valid State command should be applied before the error");
        assert_eq!(playstate.position, Some(42.0));
        assert_eq!(playstate.paused, Some(true));
        assert_eq!(application.session().username(), None);
    }

    #[test]
    fn application_commands_redact_all_credential_bearing_inputs() {
        let secret = "never-print-this";
        let commands = [
            ClientCommand::request_controller_auth("room-a", secret),
            ClientCommand::ReceiveProtocolLine {
                line: format!(r#"{{\"Hello\":{{\"password\":\"{secret}\"}}}}"#),
                received_at_seconds: 1.0,
            },
            ClientCommand::OpenMedia {
                path: format!("https://plex.invalid/video?X-Plex-Token={secret}"),
            },
        ];

        for debug in commands.iter().map(|command| format!("{command:?}")) {
            assert!(!debug.contains(secret));
        }
        assert!(format!("{:?}", &commands[0]).contains("<redacted>"));
        assert!(format!("{:?}", &commands[2]).contains("<redacted>"));
    }

    #[test]
    fn application_failure_event_redacts_tokenized_media_target() {
        let secret = "application-player-error-token-canary";
        let target = format!("https://plex.invalid/video?X-Plex-Token={secret}");
        let player = TestPlayer {
            open_error: Some(format!("mpv could not load {target}")),
            ..TestPlayer::default()
        };
        let mut application = ClientApplication::new(ClientSession::default(), player);

        let events = application.dispatch(ClientCommand::OpenMedia { path: target });
        let debug = format!("{events:?}");
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::OperationFailed { message, .. }
                if message.contains(sorotte_secret::REDACTED_SECRET)
                    && !message.contains(secret)
        )));
    }

    #[test]
    fn application_failure_event_debug_redacts_whitespace_reflected_password() {
        const MARKER: &str = "application-reflected-password-canary";
        let event = ClientEvent::OperationFailed {
            operation: "receive-protocol-line",
            message: format!(r#"server error: Not JSON: {{"password" : "{MARKER}"}}"#),
        };

        let debug = format!("{event:?}");

        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(MARKER));
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

    #[tokio::test]
    async fn worker_wait_keeps_player_integrations_maintained() {
        let mut application =
            ClientApplication::new(ClientSession::default(), TestPlayer::default());
        let maintenance_calls =
            application.with_player_io(|player| Arc::clone(&player.maintenance_calls));
        let (release_worker, wait_for_release) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            wait_for_release
                .await
                .expect("maintenance observer should release the worker");
            42_u8
        });
        let observer = tokio::spawn(async move {
            while maintenance_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            let _ = release_worker.send(());
        });

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            application.await_worker_with_player_integration_maintenance(worker),
        )
        .await
        .expect("maintenance-aware worker should not stall")
        .expect("maintenance-aware worker should join");

        assert_eq!(result, 42);
        observer.await.expect("maintenance observer should finish");
        assert!(
            application
                .with_player_io(|player| { player.maintenance_calls.load(Ordering::SeqCst) })
                >= 1,
            "maintenance must run before the pending worker is released"
        );
    }
}
