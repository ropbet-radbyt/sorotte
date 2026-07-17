#[cfg(test)]
mod command_ack_tests;
mod player_adapter;
mod state;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    process,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
        mpsc::TryRecvError,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sorotte_player_api::{
    LocalFileUpdate, PlayerCommandFailureKind, PlayerCommandId, PlayerCommandProgress,
    PlayerCommandResult, PlayerError, PlayerMediaGeneration, PlayerMediaLoadFailureKind,
    PlayerMediaLoadOutcome, PlayerObservationTimestamp, PlayerPlayIntent,
    PlayerPlaybackTelemetryUpdate, PlayerSeekableRange, PlayerTimelineKind, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};

use crate::bridge::{SorotteBridgeFailure, SorotteBridgeFailureKind, SorotteBridgeHealth};
use crate::bridge_resource::materialize_bundled_sorotte_bridge;
use crate::constants::*;
#[cfg(test)]
use crate::ipc::MpvJsonIpcTransport;
use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use crate::legacy_ui::{
    LegacySyncplayOsdKind, LegacySyncplayUiSettings, sanitize_legacy_syncplay_script_message_text,
};
use crate::live_probe::{
    PendingYtdlLiveProbe, YtdlLiveMetadataCapability, YtdlLiveProbeOutcome, spawn_ytdl_live_probe,
    youtube_live_probe_execution_target,
};

use self::state::MpvObservedState;

const PAUSED_POSITION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES: usize = 64;
const MAX_PENDING_COMMAND_PROGRESS_UPDATES: usize = 128;
const PLAYER_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const PLAYER_LOAD_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PLAYBACK_ADVANCEMENT_EPSILON_SECONDS: f64 = 0.01;
const YTDL_LIVE_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const LEGACY_SYNCPLAYINTF_OWNER_LEASE_MS: u64 = 2_000;
const LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL: Duration = Duration::from_secs(2);
const LEGACY_SYNCPLAYINTF_RUNTIME_RECOVERY_ATTEMPTS: usize = 3;
const LEGACY_SYNCPLAYINTF_DISCOVERY_ATTEMPTS: usize = 3;
const LEGACY_SYNCPLAYINTF_REGISTRATION_ATTEMPTS: usize = 20;
const LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW: Duration = Duration::from_millis(2_500);
const LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_INTERVAL: Duration = Duration::from_millis(25);
static NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT: AtomicU64 = AtomicU64::new(1);
static LEGACY_SYNCPLAYINTF_OWNER_ID: LazyLock<String> = LazyLock::new(|| {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("sorotte-{}-{started_at}", process::id())
});

fn uses_network_media_options(path: &str) -> bool {
    let Some((scheme, _)) = path.trim().split_once("://") else {
        return false;
    };
    !scheme.eq_ignore_ascii_case("file")
}

fn classify_sorotte_bridge_configuration_failure(
    reason: &str,
    acknowledged_rejection: bool,
) -> SorotteBridgeFailureKind {
    let normalized = reason.to_ascii_lowercase();
    if normalized.contains("another sorotte owner") || normalized.contains("live bridge lease") {
        SorotteBridgeFailureKind::LeaseBusy
    } else if acknowledged_rejection {
        SorotteBridgeFailureKind::SettingsRejected
    } else if normalized.contains("json ipc")
        || normalized.contains("not connected")
        || normalized.contains("command queue")
    {
        SorotteBridgeFailureKind::IpcCommand
    } else {
        SorotteBridgeFailureKind::AcknowledgementTimeout
    }
}

#[derive(Debug)]
struct PendingTrackedCommand {
    id: PlayerCommandId,
    media_generation: Option<PlayerMediaGeneration>,
    accepted_at: Option<Instant>,
    deferred_result: Option<PlayerCommandResult>,
    kind: TrackedCommandKind,
}

#[derive(Debug)]
enum TrackedCommandKind {
    Load {
        file_loaded: bool,
        ready: bool,
    },
    Seek {
        target_seconds: f64,
        seeking_finished: bool,
        position_in_tolerance: bool,
    },
    Pause {
        logical_pause_observed: bool,
    },
    Play {
        intent: PlayerPlayIntent,
        restart_sequence_baseline: u64,
        position_baseline: Option<f64>,
        logical_play_observed: bool,
        cache_clear_observed: bool,
        restart_observed: bool,
        forward_advancement_observed: bool,
    },
}

impl TrackedCommandKind {
    fn timeout(&self) -> Duration {
        match self {
            Self::Load { .. } => PLAYER_LOAD_COMMAND_TIMEOUT,
            Self::Seek { .. } | Self::Pause { .. } | Self::Play { .. } => PLAYER_COMMAND_TIMEOUT,
        }
    }

    fn completed(&self) -> bool {
        match self {
            Self::Load { file_loaded, ready } => *file_loaded && *ready,
            Self::Seek {
                seeking_finished,
                position_in_tolerance,
                ..
            } => *seeking_finished && *position_in_tolerance,
            Self::Pause {
                logical_pause_observed,
            } => *logical_pause_observed,
            Self::Play {
                intent,
                logical_play_observed,
                cache_clear_observed,
                restart_observed,
                forward_advancement_observed,
                ..
            } => {
                let restart_satisfied =
                    matches!(intent, PlayerPlayIntent::Resume) || *restart_observed;
                *logical_play_observed
                    && *cache_clear_observed
                    && restart_satisfied
                    && *forward_advancement_observed
            }
        }
    }

    fn is_load_seek_or_play(&self) -> bool {
        matches!(
            self,
            Self::Load { .. } | Self::Seek { .. } | Self::Play { .. }
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum TrackedCommandObservation {
    FileLoaded,
    Phase(PlayerTransportPhase),
    LogicalPause(bool),
    CachePause(bool),
    Seeking(bool),
    Position(f64),
    PlaybackRestart(u64),
}

#[derive(Debug, Clone, Copy)]
enum TrackedCommandSupersession {
    Load,
    Seek,
    PauseOrPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MpvLoadfileOptionsSyntax {
    Legacy,
    InsertionIndex,
    Unknown,
}

pub struct MpvAdapter {
    paused: bool,
    logical_pause_explicit: bool,
    position_seconds: f64,
    playback_rate: f64,
    paused_for_cache: bool,
    cache_buffering_percent: Option<f64>,
    muted: bool,
    volume: Option<f64>,
    deinterlace: bool,
    keepaspect: bool,
    keepaspect_window: bool,
    fullscreen: bool,
    ontop: bool,
    border: bool,
    force_window: bool,
    keep_open: bool,
    keep_open_pause: bool,
    cursor_autohide_fs_only: bool,
    stop_screensaver: bool,
    sub_visibility: bool,
    osd_bar: bool,
    window_maximized: bool,
    window_minimized: bool,
    current_path: Option<String>,
    network_media_options: BTreeMap<String, String>,
    loadfile_options_syntax: Option<MpvLoadfileOptionsSyntax>,
    mpv_version: Option<(u64, u64)>,
    pending_local_file_update: Option<LocalFileUpdate>,
    pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
    pending_transport_telemetry_updates: VecDeque<PlayerTransportTelemetryUpdate>,
    pending_tracked_commands: VecDeque<PendingTrackedCommand>,
    pending_command_progress_updates: VecDeque<PlayerCommandProgress>,
    pending_media_load_outcomes: VecDeque<PlayerMediaLoadOutcome>,
    pending_chat_requests: VecDeque<String>,
    pending_load_request: Option<String>,
    last_polled_local_file_update: Option<LocalFileUpdate>,
    last_paused_position_poll_at: Option<Instant>,
    observed_state: MpvObservedState,
    observers_registered: bool,
    transport_observers_registered: bool,
    observation_clock_origin: Instant,
    next_media_generation: u64,
    active_media_generation: Option<PlayerMediaGeneration>,
    pending_load_generation: Option<PlayerMediaGeneration>,
    active_playlist_entry_id: Option<u64>,
    playlist_entry_generations: HashMap<u64, PlayerMediaGeneration>,
    transport_phase: PlayerTransportPhase,
    active_file_loaded: bool,
    active_generation_has_restarted: bool,
    timeline_kind: PlayerTimelineKind,
    ytdl_is_live: bool,
    ytdl_is_live_metadata_generation: Option<PlayerMediaGeneration>,
    latest_cached_seekable_window: Option<PlayerSeekableRange>,
    path_metadata_generation: Option<PlayerMediaGeneration>,
    duration_metadata_generation: Option<PlayerMediaGeneration>,
    ytdl_live_probe_executable: Option<PathBuf>,
    ytdl_live_probe_path_prefixes: Vec<PathBuf>,
    ytdl_live_probe_identity: Option<(PlayerMediaGeneration, String)>,
    pending_ytdl_live_probe: Option<PendingYtdlLiveProbe>,
    playback_restart_sequence: u64,
    next_command_id: u64,
    legacy_syncplay_ui_settings: LegacySyncplayUiSettings,
    last_simulated_legacy_syncplay_osd_message: Option<(String, LegacySyncplayOsdKind)>,
    legacy_syncplay_osd_placement_restore: Option<(String, i64)>,
    legacy_syncplayintf_script_loaded: bool,
    legacy_syncplayintf_options_applied: bool,
    legacy_syncplayintf_script_name: String,
    legacy_syncplayintf_bridge_instance_id: Option<String>,
    legacy_syncplayintf_owner_id: String,
    legacy_syncplayintf_attachment_id: String,
    legacy_syncplayintf_next_options_generation: u64,
    legacy_syncplayintf_pending_options_generation: Option<u64>,
    legacy_syncplayintf_acknowledged_options_generation: Option<u64>,
    legacy_syncplayintf_options_ack_error: Option<String>,
    legacy_syncplayintf_next_ping_nonce: u64,
    legacy_syncplayintf_pending_ping_nonce: Option<u64>,
    legacy_syncplayintf_last_heartbeat_at: Option<Instant>,
    legacy_syncplayintf_last_discovery_at: Option<Instant>,
    legacy_syncplayintf_lease_reacquire_required: bool,
    legacy_syncplayintf_runtime_rediscovery_required: bool,
    legacy_syncplayintf_runtime_recovery_attempts: usize,
    legacy_syncplayintf_runtime_recovery_failure: Option<SorotteBridgeFailure>,
    sorotte_bridge_health: SorotteBridgeHealth,
    pending_sorotte_bridge_health_transitions: VecDeque<SorotteBridgeHealth>,
    ipc_endpoint: Option<PathBuf>,
    simulation_mode: bool,
    ipc_client: Option<MpvJsonIpcClient>,
    pending_ipc_connection_events: VecDeque<MpvIpcConnectionEvent>,
}

impl MpvAdapter {
    pub fn with_json_ipc(path: impl AsRef<Path>) -> Result<Self, PlayerError> {
        let mut adapter = Self::default();
        adapter.connect_json_ipc(path)?;
        Ok(adapter)
    }

    pub fn connect_json_ipc(&mut self, path: impl AsRef<Path>) -> Result<(), PlayerError> {
        let endpoint = path.as_ref().to_path_buf();
        let mut client =
            MpvJsonIpcClient::connect(&endpoint).map_err(PlayerError::OperationFailed)?;
        let version = match client.get_property_string_classified(MPV_PROPERTY_VERSION) {
            Ok(version) => version,
            Err(error) if error.is_property_unavailable() => None,
            Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
        };
        self.release_sorotte_bridge_best_effort();
        self.collect_ipc_connection_events();
        self.simulation_mode = false;
        self.ipc_client = Some(client);
        self.ipc_endpoint = Some(endpoint);
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
        self.observers_registered = false;
        self.transport_observers_registered = false;
        self.loadfile_options_syntax = None;
        self.mpv_version = version
            .as_deref()
            .and_then(Self::parse_mpv_major_minor_version);
        self.legacy_syncplay_osd_placement_restore = None;
        Ok(())
    }

    fn reset_legacy_syncplayintf_attachment_for_new_ipc(&mut self) {
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_last_discovery_at = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        // Health transitions are scoped to one IPC endpoint and must never outlive it.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        self.pending_chat_requests.clear();
        let connection_generation = self
            .ipc_client
            .as_ref()
            .map(MpvJsonIpcClient::generation)
            .unwrap_or_else(|| NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT.fetch_add(1, Ordering::Relaxed));
        self.legacy_syncplayintf_attachment_id = format!(
            "{}-{connection_generation}",
            self.legacy_syncplayintf_owner_id
        );
    }

    pub fn is_connected(&self) -> bool {
        self.ipc_client
            .as_ref()
            .is_some_and(MpvJsonIpcClient::is_healthy)
    }

    pub(crate) fn simulated() -> Self {
        Self {
            simulation_mode: true,
            ..Self::default()
        }
    }

    /// Builds a connected adapter whose test transport accepts mpv commands but never emits the
    /// Lua settings acknowledgement.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_unacknowledging_syncplayintf_test_ipc(settings: LegacySyncplayUiSettings) -> Self {
        Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            ipc_client: Some(crate::test_support::unacknowledging_syncplayintf_client()),
            ..Self::default()
        }
    }

    /// Builds a connected adapter whose test transport accepts bridge discovery commands but
    /// never emits the canonical pong.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_undiscoverable_sorotte_bridge_test_ipc(settings: LegacySyncplayUiSettings) -> Self {
        let mut adapter = Self {
            legacy_syncplay_ui_settings: settings,
            ipc_client: Some(crate::test_support::undiscoverable_syncplayintf_client()),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    /// Builds a connected adapter whose fake mpv rejects only canonical bridge discovery while
    /// leaving the core JSON IPC transport healthy.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_rejected_sorotte_bridge_discovery_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> Self {
        let mut adapter = Self {
            legacy_syncplay_ui_settings: settings,
            ipc_client: Some(crate::test_support::rejecting_syncplayintf_discovery_client()),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    /// Builds a ready bridge attachment and returns a counter incremented when its terminal
    /// release reaches the fake IPC transport.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_release_recording_sorotte_bridge_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let (ipc_client, release_count) =
            crate::test_support::release_recording_syncplayintf_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, release_count)
    }

    /// Builds a ready bridge attachment whose terminal cleanup commands are recorded in order.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_cleanup_recording_sorotte_bridge_test_ipc(
        settings: LegacySyncplayUiSettings,
        osd_placement_restore: Option<(String, i64)>,
    ) -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let (ipc_client, commands) = crate::test_support::cleanup_recording_syncplayintf_client();
        let adapter = Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplay_osd_placement_restore: osd_placement_restore,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            ipc_client: Some(ipc_client),
            ..Self::default()
        };
        (adapter, commands)
    }

    /// Builds a ready simulated bridge over connected IPC that rejects the first active-network
    /// option write while accepting a later retry.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_first_active_network_option_rejection_test_ipc(
        settings: LegacySyncplayUiSettings,
    ) -> Self {
        Self {
            legacy_syncplay_ui_settings: settings,
            legacy_syncplayintf_script_loaded: true,
            legacy_syncplayintf_options_applied: true,
            legacy_syncplayintf_bridge_instance_id: Some("test-bridge".to_owned()),
            legacy_syncplayintf_acknowledged_options_generation: Some(1),
            sorotte_bridge_health: SorotteBridgeHealth::Ready,
            simulation_mode: true,
            ipc_client: Some(crate::test_support::reject_first_active_network_option_client()),
            ..Self::default()
        }
    }

    /// Marks a feature-gated fake IPC client unhealthy so higher-layer tests can distinguish a
    /// fatal player transport loss from optional bridge degradation.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn mark_test_ipc_unhealthy(&mut self, reason: impl Into<String>) {
        if let Some(client) = self.ipc_client.as_mut() {
            client.mark_unhealthy_for_test(reason);
        }
        self.collect_ipc_connection_events();
    }

    pub fn take_ipc_connection_events(&mut self) -> Vec<MpvIpcConnectionEvent> {
        self.collect_ipc_connection_events();
        self.pending_ipc_connection_events.drain(..).collect()
    }

    fn collect_ipc_connection_events(&mut self) {
        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };
        self.pending_ipc_connection_events
            .extend(ipc_client.take_connection_events());
    }

    pub fn current_path(&self) -> Option<&str> {
        self.current_path.as_deref()
    }

    /// Selects the yt-dlp executable used to recover live-timeline metadata
    /// from stock mpv releases older than 0.39.
    ///
    /// When unset, the bounded probe tries `yt-dlp` and then `youtube-dl`
    /// through the process `PATH`. A configured path is authoritative and is
    /// never silently replaced with another executable.
    pub fn configure_ytdl_live_probe_executable(&mut self, executable: Option<PathBuf>) {
        self.configure_ytdl_live_probe_environment(executable, Vec::new());
    }

    /// Configures the bounded legacy live probe without changing the process
    /// environment inherited by Sorotte itself.
    pub fn configure_ytdl_live_probe_environment(
        &mut self,
        executable: Option<PathBuf>,
        mut path_prefixes: Vec<PathBuf>,
    ) {
        if let Some(parent) = executable.as_deref().and_then(Path::parent)
            && !parent.as_os_str().is_empty()
            && !path_prefixes.iter().any(|prefix| prefix == parent)
        {
            path_prefixes.insert(0, parent.to_path_buf());
        }
        self.ytdl_live_probe_executable = executable;
        self.ytdl_live_probe_path_prefixes = path_prefixes;
        if self.pending_ytdl_live_probe.is_none()
            && self.ytdl_live_probe_identity.is_none()
            && let (Some(generation), Some(target)) =
                (self.active_media_generation, self.current_path.clone())
        {
            self.maybe_start_ytdl_live_probe(generation, &target);
        }
    }

    /// Configures options that mpv should apply only while playing network media.
    ///
    /// The options are attached to Sorotte-issued `loadfile` commands as mpv
    /// per-file options. mpv restores the user's prior values when that media
    /// ends, so a later local file keeps its normal mpv/user cache policy.
    pub fn configure_network_media_options<I, K, V>(&mut self, options: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.network_media_options = options
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
    }

    /// Applies the configured network options to an already-active network file.
    ///
    /// mpv's `file-local-options` namespace snapshots the prior option values
    /// and restores them when the file ends. This is useful when Sorotte
    /// attaches to an existing mpv session or changes settings in place. Local
    /// files are deliberately left untouched.
    pub fn apply_network_media_options_to_active_media(&mut self) -> Result<(), PlayerError> {
        // `current_path` may describe a requested load or a prior externally
        // replaced playlist entry. An attached mpv is authoritative; the cache
        // is safe only for simulation or other no-IPC operation.
        let active_path = match self.ipc_client.as_mut() {
            Some(client) => match client.get_property_string_classified(MPV_PROPERTY_PATH) {
                Ok(path) => path,
                Err(error) if error.is_property_unavailable() => None,
                Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
            },
            None => self.current_path.clone(),
        };
        let Some(active_path) = active_path else {
            return Ok(());
        };
        if !uses_network_media_options(&active_path) {
            return Ok(());
        }

        for (name, value) in self.network_media_options.clone() {
            self.send_ipc_command_if_attached(json!([
                MPV_COMMAND_SET_PROPERTY,
                format!("file-local-options/{name}"),
                value
            ]))?;
        }
        Ok(())
    }

    fn network_media_options_map(&self) -> serde_json::Map<String, Value> {
        self.network_media_options
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect()
    }

    fn detect_loadfile_options_syntax(&mut self) -> MpvLoadfileOptionsSyntax {
        if let Some(syntax) = self.loadfile_options_syntax {
            return syntax;
        }
        let syntax = self
            .mpv_version
            .map(Self::loadfile_options_syntax_from_version_components)
            .or_else(|| {
                self.ipc_client
                    .as_mut()
                    .and_then(|client| {
                        client
                            .get_property_string(MPV_PROPERTY_VERSION)
                            .ok()
                            .flatten()
                    })
                    .as_deref()
                    .and_then(Self::loadfile_options_syntax_from_version)
            })
            .unwrap_or(MpvLoadfileOptionsSyntax::Unknown);
        self.loadfile_options_syntax = Some(syntax);
        syntax
    }

    fn loadfile_options_syntax_from_version(version: &str) -> Option<MpvLoadfileOptionsSyntax> {
        Self::parse_mpv_major_minor_version(version)
            .map(Self::loadfile_options_syntax_from_version_components)
    }

    fn parse_mpv_major_minor_version(version: &str) -> Option<(u64, u64)> {
        version
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .filter(|part| part.contains('.'))
            .find_map(|part| {
                let mut components = part.split('.');
                Some((
                    components.next()?.parse::<u64>().ok()?,
                    components.next()?.parse::<u64>().ok()?,
                ))
            })
    }

    fn loadfile_options_syntax_from_version_components(
        (major, minor): (u64, u64),
    ) -> MpvLoadfileOptionsSyntax {
        if major > 0 || minor >= 38 {
            MpvLoadfileOptionsSyntax::InsertionIndex
        } else {
            MpvLoadfileOptionsSyntax::Legacy
        }
    }

    fn send_network_media_loadfile(&mut self, path: &str) -> Result<(), PlayerError> {
        let options = Value::Object(self.network_media_options_map());
        let modern_command = || {
            json!([
                MPV_COMMAND_LOADFILE,
                path,
                MPV_LOADFILE_REPLACE,
                -1,
                options.clone()
            ])
        };
        let legacy_command = || {
            json!([
                MPV_COMMAND_LOADFILE,
                path,
                MPV_LOADFILE_REPLACE,
                options.clone()
            ])
        };

        match self.detect_loadfile_options_syntax() {
            MpvLoadfileOptionsSyntax::InsertionIndex => {
                self.send_ipc_command_if_attached(modern_command())
            }
            MpvLoadfileOptionsSyntax::Legacy => self.send_ipc_command_if_attached(legacy_command()),
            MpvLoadfileOptionsSyntax::Unknown => {
                let Some(ipc_client) = self.ipc_client.as_mut() else {
                    return self.send_ipc_command_if_attached(modern_command());
                };
                let modern_result =
                    ipc_client.send_compatibility_probe_expect_success(modern_command());
                self.drain_ipc_events_if_attached();
                match modern_result {
                    Ok(()) => {
                        self.loadfile_options_syntax =
                            Some(MpvLoadfileOptionsSyntax::InsertionIndex);
                        Ok(())
                    }
                    Err(primary_error) if primary_error.is_server_rejection() => {
                        let primary_message = primary_error.message().to_owned();
                        let result = self.send_ipc_command_if_attached(legacy_command());
                        if result.is_ok() {
                            self.loadfile_options_syntax = Some(MpvLoadfileOptionsSyntax::Legacy);
                            return Ok(());
                        }
                        Err(PlayerError::OperationFailed(format!(
                            "mpv loadfile compatibility probe was rejected ({primary_message}); legacy fallback failed: {}",
                            result.expect_err("failed result must contain its error")
                        )))
                    }
                    Err(primary_error) => Err(PlayerError::OperationFailed(
                        primary_error.message().to_owned(),
                    )),
                }
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_seconds
    }

    pub fn playback_rate(&self) -> f64 {
        if self.playback_rate == 0.0 {
            1.0
        } else {
            self.playback_rate
        }
    }

    pub fn paused_for_cache(&self) -> bool {
        self.paused_for_cache
    }

    pub fn cache_buffering_percent(&self) -> Option<f64> {
        self.cache_buffering_percent
    }

    pub fn media_generation(&self) -> Option<PlayerMediaGeneration> {
        self.pending_load_generation
            .or(self.active_media_generation)
    }

    pub fn transport_phase(&self) -> PlayerTransportPhase {
        self.transport_phase
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn volume(&self) -> f64 {
        self.volume.unwrap_or(100.0)
    }

    pub fn deinterlace(&self) -> bool {
        self.deinterlace
    }

    pub fn keepaspect(&self) -> bool {
        self.keepaspect
    }

    pub fn keepaspect_window(&self) -> bool {
        self.keepaspect_window
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn ontop(&self) -> bool {
        self.ontop
    }

    pub fn border(&self) -> bool {
        self.border
    }

    pub fn force_window(&self) -> bool {
        self.force_window
    }

    pub fn keep_open(&self) -> bool {
        self.keep_open
    }

    pub fn keep_open_pause(&self) -> bool {
        self.keep_open_pause
    }

    pub fn cursor_autohide_fs_only(&self) -> bool {
        self.cursor_autohide_fs_only
    }

    pub fn stop_screensaver(&self) -> bool {
        self.stop_screensaver
    }

    pub fn sub_visibility(&self) -> bool {
        self.sub_visibility
    }

    pub fn osd_bar(&self) -> bool {
        self.osd_bar
    }

    pub fn window_maximized(&self) -> bool {
        self.window_maximized
    }

    pub fn window_minimized(&self) -> bool {
        self.window_minimized
    }

    pub fn queue_local_file_update(&mut self, update: LocalFileUpdate) {
        self.pending_local_file_update = Some(update);
    }

    pub fn legacy_syncplay_ui_settings(&self) -> &LegacySyncplayUiSettings {
        &self.legacy_syncplay_ui_settings
    }

    pub fn last_simulated_legacy_syncplay_osd_message(
        &self,
    ) -> Option<&(String, LegacySyncplayOsdKind)> {
        self.last_simulated_legacy_syncplay_osd_message.as_ref()
    }

    pub fn legacy_syncplayintf_options_ready(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_bridge_instance_id.is_some()
            && self.legacy_syncplayintf_options_applied
            && self
                .legacy_syncplayintf_pending_options_generation
                .is_none()
    }

    pub fn legacy_syncplayintf_script_loaded(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
    }

    pub fn apply_pending_legacy_syncplayintf_options(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the syncplayintf bridge is not loaded".to_owned(),
            ));
        }
        if self.legacy_syncplayintf_options_applied {
            return Ok(());
        }
        self.send_legacy_syncplayintf_options_if_loaded()
    }

    pub fn legacy_syncplay_osd_placement_restore(&self) -> Option<(String, i64)> {
        self.legacy_syncplay_osd_placement_restore.clone()
    }

    pub fn set_legacy_syncplay_osd_placement_restore(&mut self, restore: Option<(String, i64)>) {
        self.legacy_syncplay_osd_placement_restore = restore;
    }

    pub fn set_property_string(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn set_property_i64(&mut self, name: &str, value: i64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn show_text(
        &mut self,
        text: &str,
        duration_ms: u64,
        level: i64,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SHOW_TEXT, text, duration_ms, level]))
    }

    pub fn load_legacy_syncplayintf_script(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), PlayerError> {
        if self.ipc_client.is_none() {
            if self.simulation_mode {
                self.legacy_syncplayintf_script_loaded = true;
                self.legacy_syncplayintf_bridge_instance_id =
                    Some("simulated-sorotte-syncplayintf".to_owned());
                self.legacy_syncplayintf_options_applied = true;
            }
            return Ok(());
        }

        if self.discover_legacy_syncplayintf_bridge(false)? {
            self.try_send_legacy_syncplayintf_options_if_pending();
            return Ok(());
        }

        let script_path = path.as_ref().to_string_lossy().into_owned();
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_LOAD_SCRIPT, script_path]))?;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        if !self.discover_legacy_syncplayintf_bridge(true)? {
            return Err(PlayerError::OperationFailed(
                "loaded the Sorotte syncplayintf resource, but its stable bridge did not answer discovery"
                    .to_owned(),
            ));
        }
        self.try_send_legacy_syncplayintf_options_if_pending();
        Ok(())
    }

    pub fn configure_legacy_syncplay_ui_settings(
        &mut self,
        settings: LegacySyncplayUiSettings,
    ) -> Result<(), PlayerError> {
        let syncplayintf_options_changed = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_differ(&settings);
        let placement_available = self.ipc_client.is_some() || self.simulation_mode;
        if placement_available && settings.should_move_osd() {
            if self.legacy_syncplay_osd_placement_restore.is_none() {
                let restore = match self.ipc_client.as_mut() {
                    Some(client) => {
                        let align = client
                            .get_property_string(MPV_PROPERTY_OSD_ALIGN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical alignment".to_owned(),
                                )
                            })?;
                        let margin = client
                            .get_property_i64(MPV_PROPERTY_OSD_MARGIN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical margin".to_owned(),
                                )
                            })?;
                        (align, margin)
                    }
                    None => ("top".to_owned(), 0),
                };
                self.legacy_syncplay_osd_placement_restore = Some(restore);
            }
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, "bottom")?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, settings.chat_osd_margin)?;
        } else if placement_available
            && let Some((align, margin)) =
                self.legacy_syncplay_osd_placement_restore.as_ref().cloned()
        {
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, &align)?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, margin)?;
            self.legacy_syncplay_osd_placement_restore = None;
        }
        self.legacy_syncplay_ui_settings = settings;
        if syncplayintf_options_changed {
            let runtime_bridge_was_active = matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
            );
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
            if runtime_bridge_was_active {
                self.legacy_syncplayintf_runtime_recovery_attempts = 0;
                self.legacy_syncplayintf_runtime_recovery_failure = None;
                self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::AcknowledgementTimeout,
                    "updated Chat/OSD settings are awaiting bridge acknowledgement",
                    false,
                );
                self.attempt_sorotte_bridge_runtime_recovery();
            } else {
                self.try_send_legacy_syncplayintf_options_if_pending();
            }
        }
        Ok(())
    }

    pub fn configure_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn retry_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn sorotte_bridge_health(&self) -> SorotteBridgeHealth {
        self.sorotte_bridge_health.clone()
    }

    /// Returns the exact settings generation acknowledged by the current bridge attachment.
    pub fn sorotte_bridge_acknowledged_generation(&self) -> Option<u64> {
        self.legacy_syncplayintf_options_applied
            .then_some(self.legacy_syncplayintf_acknowledged_options_generation)
            .flatten()
    }

    /// Advances bounded bridge maintenance and returns the oldest unconsumed health transition.
    ///
    /// Bridge transitions are independent of core mpv JSON IPC health. A `Recovering` or
    /// `Degraded` transition gates player chat and causes OSD output to use mpv's `show-text`, but
    /// does not detach the adapter or make playback commands unavailable.
    pub fn take_sorotte_bridge_health_transition(&mut self) -> Option<SorotteBridgeHealth> {
        self.maintain_legacy_syncplayintf_lease();
        self.pending_sorotte_bridge_health_transitions.pop_front()
    }

    pub fn mark_sorotte_bridge_degraded(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.degrade_sorotte_bridge(kind, reason)
    }

    fn configure_bundled_sorotte_bridge_inner(
        &mut self,
        retry_window: Duration,
    ) -> SorotteBridgeHealth {
        let bridge_requested = self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge();
        if !bridge_requested && !self.legacy_syncplayintf_script_loaded {
            return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        }

        if !self.legacy_syncplayintf_script_loaded {
            match self.discover_loaded_legacy_syncplayintf_script() {
                Ok(true) => {}
                Ok(false) if bridge_requested => {
                    let script_path = match materialize_bundled_sorotte_bridge() {
                        Ok(path) => path,
                        Err(error) => {
                            return self.degrade_sorotte_bridge(
                                SorotteBridgeFailureKind::ResourceMaterialization,
                                format!(
                                    "failed to materialize Sorotte's bundled mpv bridge: {error}"
                                ),
                            );
                        }
                    };
                    if let Err(error) = self.load_legacy_syncplayintf_script(&script_path) {
                        return self.degrade_sorotte_bridge(
                            SorotteBridgeFailureKind::ScriptLoad,
                            format!(
                                "failed to load Sorotte's bundled mpv bridge from '{}': {error}",
                                script_path.display()
                            ),
                        );
                    }
                }
                Ok(false) => {
                    return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
                }
                Err(error) => {
                    return self.degrade_sorotte_bridge(
                        SorotteBridgeFailureKind::Discovery,
                        format!("failed to discover Sorotte's mpv bridge: {error}"),
                    );
                }
            }
        }

        let deadline = Instant::now() + retry_window;
        let mut last_acknowledged_error = None;
        let last_error = loop {
            let error = match self.apply_pending_legacy_syncplayintf_options() {
                Ok(()) if self.legacy_syncplayintf_options_ready() => {
                    let health = if bridge_requested {
                        SorotteBridgeHealth::Ready
                    } else {
                        SorotteBridgeHealth::Disabled
                    };
                    return self.set_sorotte_bridge_health(health);
                }
                Ok(()) => {
                    "Sorotte's mpv bridge did not report that its settings are ready".to_owned()
                }
                Err(error) => error.to_string(),
            };
            if let Some(acknowledged_error) = self.legacy_syncplayintf_options_ack_error.clone() {
                last_acknowledged_error = Some(acknowledged_error);
            }
            if Instant::now() >= deadline {
                break error;
            }
            std::thread::sleep(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_INTERVAL);
        };

        let acknowledged_error = self
            .legacy_syncplayintf_options_ack_error
            .clone()
            .or(last_acknowledged_error);
        let reason = acknowledged_error.clone().unwrap_or(last_error);
        let kind =
            classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some());
        self.degrade_sorotte_bridge(kind, reason)
    }

    fn set_sorotte_bridge_health(&mut self, health: SorotteBridgeHealth) -> SorotteBridgeHealth {
        if self.sorotte_bridge_health == health {
            return health;
        }
        self.sorotte_bridge_health = health.clone();
        if self.pending_sorotte_bridge_health_transitions.back() != Some(&health) {
            self.pending_sorotte_bridge_health_transitions
                .push_back(health.clone());
        }
        if matches!(
            health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Disabled
        ) {
            self.legacy_syncplayintf_runtime_rediscovery_required = false;
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
            self.legacy_syncplayintf_runtime_recovery_failure = None;
        }
        health
    }

    fn degrade_sorotte_bridge(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_lease_reacquire_required =
            kind == SorotteBridgeFailureKind::LeaseBusy;
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Degraded(SorotteBridgeFailure::new(
            kind, reason,
        )))
    }

    pub fn show_syncplay_legacy_message(
        &mut self,
        message: &str,
        kind: LegacySyncplayOsdKind,
    ) -> Result<(), PlayerError> {
        if message.trim().is_empty() || !self.legacy_syncplay_ui_settings.show_osd {
            return Ok(());
        }
        if self.simulation_mode {
            self.last_simulated_legacy_syncplay_osd_message = Some((message.to_owned(), kind));
        }

        let duration_ms = match kind {
            LegacySyncplayOsdKind::Notification => {
                self.legacy_syncplay_ui_settings.notification_timeout_ms
            }
            LegacySyncplayOsdKind::Alert => self.legacy_syncplay_ui_settings.alert_timeout_ms,
        };
        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            let script_message_name = match kind {
                LegacySyncplayOsdKind::Notification => "notification-osd-neutral",
                LegacySyncplayOsdKind::Alert => "alert-osd-neutral",
            };
            match self.send_syncplayintf_script_message(
                script_message_name,
                &sanitize_legacy_syncplay_script_message_text(message),
            ) {
                Ok(()) => return Ok(()),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::IpcCommand,
                    format!("Sorotte's mpv bridge rejected {script_message_name}: {error}"),
                    true,
                ),
            }
        }
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    pub fn show_syncplay_legacy_chat_message(&mut self, message: &str) -> Result<(), PlayerError> {
        if message.trim().is_empty() {
            return Ok(());
        }

        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            match self.send_syncplayintf_script_message(
                "chat",
                &sanitize_legacy_syncplay_script_message_text(message),
            ) {
                Ok(()) => return Ok(()),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::IpcCommand,
                    format!("Sorotte's mpv bridge rejected chat output: {error}"),
                    true,
                ),
            }
        }

        let maybe_duration_ms = if self.legacy_syncplay_ui_settings.chat_output_enabled {
            Some(self.legacy_syncplay_ui_settings.chat_timeout_ms)
        } else if self.legacy_syncplay_ui_settings.show_osd {
            Some(self.legacy_syncplay_ui_settings.notification_timeout_ms)
        } else {
            None
        };

        let Some(duration_ms) = maybe_duration_ms else {
            return Ok(());
        };
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    fn send_syncplayintf_script_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            self.legacy_syncplayintf_script_name.as_str(),
            message_name,
            payload
        ]))
    }

    fn send_syncplayintf_probe_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<bool, PlayerError> {
        let result = match self.ipc_client.as_mut() {
            Some(client) => client.send_compatibility_probe_expect_success(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
                message_name,
                payload
            ])),
            None if self.simulation_mode => return Ok(true),
            None => return Err(PlayerError::NotConnected),
        };
        self.drain_ipc_events_if_attached();
        match result {
            Ok(()) => Ok(true),
            Err(error) if error.is_server_rejection() => Ok(false),
            Err(error) => Err(PlayerError::OperationFailed(error.into_message())),
        }
    }

    pub fn discover_loaded_legacy_syncplayintf_script(&mut self) -> Result<bool, PlayerError> {
        self.discover_legacy_syncplayintf_bridge(false)
    }

    fn discover_legacy_syncplayintf_bridge(
        &mut self,
        wait_for_registration: bool,
    ) -> Result<bool, PlayerError> {
        if self.simulation_mode {
            self.legacy_syncplayintf_script_loaded = true;
            self.legacy_syncplayintf_bridge_instance_id =
                Some("simulated-sorotte-syncplayintf".to_owned());
            self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
            return Ok(true);
        }

        let nonce = self.legacy_syncplayintf_next_ping_nonce;
        self.legacy_syncplayintf_next_ping_nonce = self
            .legacy_syncplayintf_next_ping_nonce
            .wrapping_add(1)
            .max(1);
        self.legacy_syncplayintf_pending_ping_nonce = Some(nonce);
        let payload = json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "nonce": nonce,
        })
        .to_string();
        let mut target_accepted_a_ping = false;
        let attempts = if wait_for_registration {
            LEGACY_SYNCPLAYINTF_REGISTRATION_ATTEMPTS
        } else {
            LEGACY_SYNCPLAYINTF_DISCOVERY_ATTEMPTS
        };
        for _ in 0..attempts {
            let ping_accepted =
                self.send_syncplayintf_probe_message(LEGACY_SYNCPLAYINTF_PING_MESSAGE, &payload)?;
            target_accepted_a_ping |= ping_accepted;
            if !ping_accepted {
                if !wait_for_registration {
                    self.legacy_syncplayintf_pending_ping_nonce = None;
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_some()
                && let Some(client) = self.ipc_client.as_mut()
            {
                let _ = client.get_property(MPV_PROPERTY_PAUSE);
                self.drain_ipc_events_if_attached();
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_none()
                && self.legacy_syncplayintf_bridge_instance_id.is_some()
            {
                self.legacy_syncplayintf_script_loaded = true;
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.legacy_syncplayintf_pending_ping_nonce = None;
        if target_accepted_a_ping {
            return Err(PlayerError::OperationFailed(
                "the stable Sorotte syncplayintf target accepted discovery messages but did not return a valid pong; refusing to load a duplicate bridge"
                    .to_owned(),
            ));
        }
        Ok(false)
    }

    fn send_legacy_syncplayintf_options_if_loaded(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the Sorotte syncplayintf bridge has not been discovered".to_owned(),
            ));
        }
        if self.simulation_mode {
            let generation = self.legacy_syncplayintf_next_options_generation;
            self.legacy_syncplayintf_next_options_generation = self
                .legacy_syncplayintf_next_options_generation
                .wrapping_add(1)
                .max(1);
            self.legacy_syncplayintf_options_applied = true;
            self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
            return Ok(());
        }

        let bridge_instance_id = self
            .legacy_syncplayintf_bridge_instance_id
            .clone()
            .ok_or_else(|| {
                PlayerError::OperationFailed(
                    "the Sorotte syncplayintf bridge instance is unknown".to_owned(),
                )
            })?;
        let generation = match self.legacy_syncplayintf_pending_options_generation {
            Some(generation) => generation,
            None => {
                let generation = self.legacy_syncplayintf_next_options_generation;
                self.legacy_syncplayintf_next_options_generation = self
                    .legacy_syncplayintf_next_options_generation
                    .wrapping_add(1)
                    .max(1);
                self.legacy_syncplayintf_pending_options_generation = Some(generation);
                generation
            }
        };
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_options_ack_error = None;
        let payload = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_payload(
                &bridge_instance_id,
                &self.legacy_syncplayintf_owner_id,
                &self.legacy_syncplayintf_attachment_id,
                generation,
                LEGACY_SYNCPLAYINTF_OWNER_LEASE_MS,
            );

        self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_SET_OPTIONS_MESSAGE, &payload)?;
        if !self.legacy_syncplayintf_options_applied
            && let Some(client) = self.ipc_client.as_mut()
        {
            let _ = client.get_property(MPV_PROPERTY_PAUSE);
            self.drain_ipc_events_if_attached();
        }
        if self.legacy_syncplayintf_options_applied {
            self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            return Ok(());
        }
        Err(PlayerError::OperationFailed(
            self.legacy_syncplayintf_options_ack_error
                .clone()
                .unwrap_or_else(|| {
                    format!(
                        "Sorotte syncplayintf did not acknowledge settings generation {generation}"
                    )
                }),
        ))
    }

    fn try_send_legacy_syncplayintf_options_if_pending(&mut self) {
        if self.legacy_syncplayintf_options_applied
            || self.legacy_syncplayintf_lease_reacquire_required
            || matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Recovering | SorotteBridgeHealth::Degraded(_)
            )
        {
            return;
        }

        let _ = self.send_legacy_syncplayintf_options_if_loaded();
    }

    fn ensure_legacy_syncplayintf_ready(&mut self) -> bool {
        self.try_send_legacy_syncplayintf_options_if_pending();
        self.legacy_syncplayintf_options_ready()
    }

    fn legacy_syncplayintf_controller_payload(&self) -> Option<String> {
        Some(
            json!({
                "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
                "bridgeInstanceId": self.legacy_syncplayintf_bridge_instance_id.as_deref()?,
                "ownerId": self.legacy_syncplayintf_owner_id.as_str(),
                "attachmentId": self.legacy_syncplayintf_attachment_id.as_str(),
            })
            .to_string(),
        )
    }

    fn begin_sorotte_bridge_runtime_recovery(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
        rediscovery_required: bool,
    ) {
        if !matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
        ) {
            return;
        }
        let reason = reason.into();
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        }
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_lease_reacquire_required = true;
        self.legacy_syncplayintf_runtime_rediscovery_required |= rediscovery_required;
        self.legacy_syncplayintf_runtime_recovery_failure =
            Some(SorotteBridgeFailure::new(kind, reason));
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Recovering);
    }

    fn attempt_sorotte_bridge_runtime_recovery(&mut self) {
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering)
            || (self.legacy_syncplayintf_runtime_recovery_attempts > 0
                && self
                    .legacy_syncplayintf_last_heartbeat_at
                    .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL))
        {
            return;
        }
        self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());

        let mut forced_failure_kind = None;
        let result = if self.legacy_syncplayintf_runtime_rediscovery_required {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {
                    self.legacy_syncplayintf_runtime_rediscovery_required = false;
                    self.send_legacy_syncplayintf_options_if_loaded()
                }
                Ok(false) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(PlayerError::OperationFailed(
                        "Sorotte's stable mpv bridge target is no longer registered".to_owned(),
                    ))
                }
                Err(error) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(error)
                }
            }
        } else {
            self.send_legacy_syncplayintf_options_if_loaded()
        };

        if result.is_ok() && self.legacy_syncplayintf_options_ready() {
            let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                SorotteBridgeHealth::Ready
            } else {
                SorotteBridgeHealth::Disabled
            };
            self.set_sorotte_bridge_health(health);
            return;
        }
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            return;
        }

        self.legacy_syncplayintf_runtime_recovery_attempts += 1;
        if let Err(error) = result {
            let acknowledged_error = self.legacy_syncplayintf_options_ack_error.clone();
            let reason = acknowledged_error
                .clone()
                .unwrap_or_else(|| error.to_string());
            let kind = forced_failure_kind.unwrap_or_else(|| {
                classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some())
            });
            self.legacy_syncplayintf_runtime_recovery_failure =
                Some(SorotteBridgeFailure::new(kind, reason));
        }

        if self.legacy_syncplayintf_runtime_recovery_attempts
            >= LEGACY_SYNCPLAYINTF_RUNTIME_RECOVERY_ATTEMPTS
        {
            let failure = self
                .legacy_syncplayintf_runtime_recovery_failure
                .clone()
                .unwrap_or_else(|| {
                    SorotteBridgeFailure::new(
                        SorotteBridgeFailureKind::AcknowledgementTimeout,
                        "Sorotte's mpv bridge did not acknowledge bounded runtime recovery",
                    )
                });
            self.degrade_sorotte_bridge(failure.kind, failure.reason);
        }
    }

    fn maintain_legacy_syncplayintf_lease(&mut self) {
        self.drain_ipc_events_if_attached();
        if matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Disabled | SorotteBridgeHealth::Degraded(_)
        ) {
            return;
        }
        if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }

        if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge()
            && self
                .legacy_syncplayintf_last_discovery_at
                .is_none_or(|last| last.elapsed() >= LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL)
        {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {}
                Ok(false) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    "Sorotte's stable mpv bridge target is no longer registered",
                    true,
                ),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    format!("failed to rediscover Sorotte's mpv bridge: {error}"),
                    true,
                ),
            }
            if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
                self.attempt_sorotte_bridge_runtime_recovery();
                return;
            }
        }

        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            self.legacy_syncplayintf_last_heartbeat_at = None;
            return;
        }
        if !self.legacy_syncplayintf_options_ready() {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::AcknowledgementTimeout,
                "Sorotte's mpv bridge lost its acknowledged runtime settings",
                false,
            );
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }
        if self
            .legacy_syncplayintf_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let Some(payload) = self.legacy_syncplayintf_controller_payload() else {
            return;
        };
        match self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_HEARTBEAT_MESSAGE, &payload)
        {
            Ok(()) if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Ready) => {
                self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            }
            Ok(()) => {
                self.legacy_syncplayintf_last_heartbeat_at = None;
                self.attempt_sorotte_bridge_runtime_recovery();
            }
            Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::IpcCommand,
                format!("failed to renew Sorotte's mpv bridge lease: {error}"),
                true,
            ),
        }
    }

    /// Queues a terminal, one-way bridge release and immediately clears local bridge state.
    ///
    /// This is a shutdown-only operation. If an IPC final write is queued, the current JSON IPC
    /// client becomes unusable; callers should invoke this immediately before detaching or
    /// replacing the adapter. Lease expiry remains the fallback when the best-effort write cannot
    /// be queued or completed.
    pub fn release_sorotte_bridge_best_effort(&mut self) {
        let mut final_commands = Vec::with_capacity(3);
        if let Some((align_y, margin_y)) = self.legacy_syncplay_osd_placement_restore.take() {
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_ALIGN_Y,
                align_y
            ]));
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_MARGIN_Y,
                margin_y
            ]));
        }
        if self.legacy_syncplayintf_script_loaded
            && let Some(payload) = self.legacy_syncplayintf_controller_payload()
        {
            final_commands.push(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                self.legacy_syncplayintf_script_name.as_str(),
                LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE,
                payload
            ]));
        }
        if !final_commands.is_empty()
            && let Some(client) = self.ipc_client.as_mut()
        {
            client.send_final_commands_best_effort(final_commands);
        }
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.pending_chat_requests.clear();
        // Release is terminal for this endpoint; queued observations are no longer actionable.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.sorotte_bridge_health = SorotteBridgeHealth::Disabled;
    }

    fn ensure_observers_registered_if_attached(&mut self) {
        if self.observers_registered {
            return;
        }
        if self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_PATH_ID, MPV_PROPERTY_PATH),
            (MPV_OBS_DURATION_ID, MPV_PROPERTY_DURATION),
            (MPV_OBS_FILE_SIZE_ID, MPV_PROPERTY_FILE_SIZE),
            (MPV_OBS_PAUSE_ID, MPV_PROPERTY_PAUSE),
            (MPV_OBS_TIME_POS_ID, MPV_PROPERTY_TIME_POS),
            (MPV_OBS_SPEED_ID, MPV_PROPERTY_SPEED),
            (MPV_OBS_PAUSED_FOR_CACHE_ID, MPV_PROPERTY_PAUSED_FOR_CACHE),
            (
                MPV_OBS_CACHE_BUFFERING_STATE_ID,
                MPV_PROPERTY_CACHE_BUFFERING_STATE,
            ),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            let registration_result = ipc_client.observe_property(observer_id, property_name);
            if registration_result.is_err() {
                return;
            }
            self.drain_ipc_events_if_attached();
        }
        self.observers_registered = true;
    }

    fn ensure_transport_observers_registered_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        if self.transport_observers_registered || self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_SEEKING_ID, MPV_PROPERTY_SEEKING),
            (MPV_OBS_SEEKABLE_ID, MPV_PROPERTY_SEEKABLE),
            (MPV_OBS_CORE_IDLE_ID, MPV_PROPERTY_CORE_IDLE),
            (
                MPV_OBS_DEMUXER_CACHE_STATE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_STATE,
            ),
            (
                MPV_OBS_DEMUXER_CACHE_IDLE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_IDLE,
            ),
            // Observe both forms: the full metadata map itself is observable
            // on the oldest supported mpv, while current mpv can report the
            // narrower subproperty without retransmitting unrelated tags.
            // Stock mpv before 0.39 does not publish this tag at all and is
            // covered by the bounded external probe below.
            (MPV_OBS_YTDL_IS_LIVE_ID, MPV_PROPERTY_YTDL_IS_LIVE),
            (MPV_OBS_METADATA_ID, MPV_PROPERTY_METADATA),
            (MPV_OBS_EOF_REACHED_ID, MPV_PROPERTY_EOF_REACHED),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            // These observations were added after the original adapter and
            // are optional on older mpv builds. A rejected property must not
            // prevent the remaining lifecycle properties from registering.
            let _ = ipc_client.observe_property(observer_id, property_name);
            self.drain_ipc_events_if_attached();
        }
        self.transport_observers_registered = true;
    }

    fn poll_ipc_local_file_update_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_local_file_update.is_some() {
            return;
        }

        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };

        let Ok(polled_update) = Self::poll_local_file_update_from_mpv(ipc_client) else {
            return;
        };
        let Some(polled_update) = polled_update else {
            return;
        };

        if self.pending_load_request.is_some() {
            self.complete_pending_load_request_from_polled_update_if_ready(polled_update);
            self.drain_ipc_events_if_attached();
            return;
        }

        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.current_path = polled_update.path.clone();
        self.path_metadata_generation = self.active_media_generation;
        self.duration_metadata_generation = self.active_media_generation;
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        if let (Some(generation), Some(target)) =
            (self.active_media_generation, polled_update.path.as_deref())
        {
            self.maybe_start_ytdl_live_probe(generation, target);
        }
        if Self::local_file_update_ready_for_sync(&polled_update) {
            self.record_local_file_update_if_changed(polled_update);
        }
        self.drain_ipc_events_if_attached();
    }

    fn poll_local_file_update_from_mpv(
        ipc_client: &mut MpvJsonIpcClient,
    ) -> Result<Option<LocalFileUpdate>, String> {
        let Some(path) = ipc_client.get_property_string(MPV_PROPERTY_PATH)? else {
            return Ok(None);
        };

        let mut local_file_update = Self::local_file_update_for_path(path.as_str());

        if let Some(duration_seconds) = ipc_client.get_property_f64(MPV_PROPERTY_DURATION)? {
            local_file_update = local_file_update.with_duration_seconds(duration_seconds);
        }

        if let Some(size_bytes) = ipc_client.get_property_u64(MPV_PROPERTY_FILE_SIZE)? {
            local_file_update = local_file_update.with_size_bytes(size_bytes);
        }

        Ok(Some(local_file_update))
    }

    fn poll_paused_position_telemetry_if_attached(&mut self) {
        if !self.paused {
            return;
        }

        let now = Instant::now();
        if self
            .last_paused_position_poll_at
            .is_some_and(|last_poll| now.duration_since(last_poll) < PAUSED_POSITION_POLL_INTERVAL)
        {
            return;
        }
        self.last_paused_position_poll_at = Some(now);

        let polled_position = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            ipc_client.get_property_f64(MPV_PROPERTY_TIME_POS)
        };
        self.drain_ipc_events_if_attached();

        let Ok(Some(position_seconds)) = polled_position else {
            return;
        };
        if !position_seconds.is_finite() || (self.position_seconds - position_seconds).abs() < 1e-6
        {
            return;
        }

        self.position_seconds = position_seconds;
        self.observed_state.position_seconds = Some(position_seconds);
        self.queue_playback_telemetry_update(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(position_seconds),
        );
        let update = self
            .transport_update()
            .with_position_seconds(position_seconds);
        self.queue_transport_telemetry_update(update);
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Position(position_seconds),
        );
    }

    fn record_local_file_update_if_changed(&mut self, update: LocalFileUpdate) {
        if self.last_polled_local_file_update.as_ref() != Some(&update) {
            self.last_polled_local_file_update = Some(update.clone());
            self.pending_local_file_update = Some(update);
        }
    }

    fn complete_pending_load_request_from_polled_update_if_ready(
        &mut self,
        polled_update: LocalFileUpdate,
    ) {
        let Some(requested_target) = self.pending_load_request.as_deref() else {
            return;
        };
        if !Self::local_file_update_matches_request(&polled_update, requested_target)
            || !Self::local_file_update_ready_for_sync(&polled_update)
        {
            return;
        }
        let requested_target = self
            .pending_load_request
            .take()
            .expect("pending request should still be present");
        let generation = self
            .pending_load_generation
            .take()
            .unwrap_or_else(|| self.allocate_media_generation());
        self.active_media_generation = Some(generation);
        self.active_file_loaded = true;
        self.current_path = polled_update.path.clone();
        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.path_metadata_generation = Some(generation);
        self.duration_metadata_generation = Some(generation);
        self.refresh_timeline_kind_from_metadata();
        self.maybe_start_ytdl_live_probe(generation, &requested_target);
        self.record_local_file_update_if_changed(polled_update.clone());
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(
                requested_target,
                polled_update.path,
            ));
        self.refresh_inferred_transport_phase();
    }

    fn queue_playback_telemetry_update(&mut self, update: PlayerPlaybackTelemetryUpdate) {
        match self.pending_playback_telemetry_update.as_mut() {
            Some(pending) => {
                if let Some(paused) = update.paused
                    && !(paused && pending.paused_for_cache == Some(true))
                {
                    pending.paused = Some(paused);
                }
                if let Some(position_seconds) = update.position_seconds {
                    pending.position_seconds = Some(position_seconds);
                }
                if let Some(playback_rate) = update.playback_rate {
                    pending.playback_rate = Some(playback_rate);
                }
                if let Some(paused_for_cache) = update.paused_for_cache {
                    pending.paused_for_cache = Some(paused_for_cache);
                    if paused_for_cache && pending.paused == Some(true) {
                        pending.paused = None;
                    }
                }
                if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                    pending.cache_buffering_percent = Some(cache_buffering_percent);
                }
            }
            None => {
                let mut update = update;
                if update.paused_for_cache == Some(true) && update.paused == Some(true) {
                    update.paused = None;
                }
                self.pending_playback_telemetry_update = Some(update);
            }
        }
    }

    fn allocate_media_generation(&mut self) -> PlayerMediaGeneration {
        let generation = self.next_media_generation.max(1);
        self.next_media_generation = generation.wrapping_add(1).max(1);
        PlayerMediaGeneration::new(generation)
    }

    fn allocate_command_id(&mut self) -> PlayerCommandId {
        let command_id = self.next_command_id.max(1);
        self.next_command_id = command_id.wrapping_add(1).max(1);
        PlayerCommandId::new(command_id)
    }

    fn register_tracked_command(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        kind: TrackedCommandKind,
    ) -> PlayerCommandId {
        let id = self.allocate_command_id();
        self.pending_tracked_commands
            .push_back(PendingTrackedCommand {
                id,
                media_generation,
                accepted_at: None,
                deferred_result: None,
                kind,
            });
        id
    }

    fn accept_tracked_command(&mut self, command_id: PlayerCommandId) {
        let Some(command) = self
            .pending_tracked_commands
            .iter_mut()
            .find(|command| command.id == command_id)
        else {
            return;
        };
        command.accepted_at = Some(Instant::now());
        let media_generation = command.media_generation;
        let deferred_result = command.deferred_result;
        self.queue_command_progress(PlayerCommandProgress::accepted(
            command_id,
            media_generation,
            Some(self.observation_timestamp()),
        ));
        if let Some(result) = deferred_result {
            self.finish_tracked_command(command_id, result);
        } else {
            self.finish_completed_tracked_commands();
        }
    }

    fn discard_unaccepted_tracked_command(&mut self, command_id: PlayerCommandId) {
        self.pending_tracked_commands
            .retain(|command| command.id != command_id);
    }

    fn queue_command_progress(&mut self, progress: PlayerCommandProgress) {
        if self.pending_command_progress_updates.len() >= MAX_PENDING_COMMAND_PROGRESS_UPDATES {
            self.pending_command_progress_updates.pop_front();
        }
        self.pending_command_progress_updates.push_back(progress);
    }

    fn finish_tracked_command(&mut self, command_id: PlayerCommandId, result: PlayerCommandResult) {
        let Some(index) = self
            .pending_tracked_commands
            .iter()
            .position(|command| command.id == command_id)
        else {
            return;
        };
        let command = self
            .pending_tracked_commands
            .remove(index)
            .expect("tracked command index should remain valid");
        let observed_position_seconds = (command.media_generation
            == self
                .active_media_generation
                .or(self.pending_load_generation))
        .then_some(self.observed_state.position_seconds)
        .flatten();
        self.queue_command_progress(PlayerCommandProgress::finished(
            command.id,
            command.media_generation,
            Some(self.observation_timestamp()),
            observed_position_seconds,
            result,
        ));
    }

    fn finish_completed_tracked_commands(&mut self) {
        let completed: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some() && command.kind.completed())
            .map(|command| command.id)
            .collect();
        for command_id in completed {
            self.finish_tracked_command(command_id, PlayerCommandResult::Completed);
        }
    }

    fn observe_tracked_commands(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observation: TrackedCommandObservation,
    ) {
        let ready_paused_observed = self.observed_state.logical_pause == Some(true)
            && self.observed_state.paused_for_cache == Some(false);
        let logical_pause_observed_independently =
            self.observed_state.paused_for_cache == Some(false);
        let pause_property_current = self.observed_state.paused == Some(true);
        for command in &mut self.pending_tracked_commands {
            if command.media_generation != media_generation {
                continue;
            }
            match (&mut command.kind, observation) {
                (
                    TrackedCommandKind::Load { file_loaded, .. },
                    TrackedCommandObservation::FileLoaded,
                ) => {
                    *file_loaded = true;
                }
                (
                    TrackedCommandKind::Load { ready, .. },
                    TrackedCommandObservation::Phase(phase),
                ) => {
                    *ready = phase == PlayerTransportPhase::Playing
                        || (phase == PlayerTransportPhase::ReadyPaused && ready_paused_observed);
                }
                (
                    TrackedCommandKind::Seek {
                        seeking_finished, ..
                    },
                    TrackedCommandObservation::Seeking(seeking),
                ) => {
                    *seeking_finished = !seeking;
                }
                (
                    TrackedCommandKind::Seek {
                        target_seconds,
                        position_in_tolerance,
                        ..
                    },
                    TrackedCommandObservation::Position(position_seconds),
                ) => {
                    *position_in_tolerance = (position_seconds - *target_seconds).abs()
                        <= crate::MPV_SEEK_COMPLETION_TOLERANCE_SECONDS;
                }
                (
                    TrackedCommandKind::Pause {
                        logical_pause_observed,
                    },
                    TrackedCommandObservation::LogicalPause(logical_pause),
                ) => {
                    *logical_pause_observed = logical_pause && logical_pause_observed_independently;
                }
                (
                    TrackedCommandKind::Pause {
                        logical_pause_observed,
                    },
                    TrackedCommandObservation::CachePause(paused_for_cache),
                ) => {
                    *logical_pause_observed = !paused_for_cache && pause_property_current;
                }
                (
                    TrackedCommandKind::Play {
                        logical_play_observed,
                        ..
                    },
                    TrackedCommandObservation::LogicalPause(logical_pause),
                ) => {
                    *logical_play_observed = !logical_pause;
                }
                (
                    TrackedCommandKind::Play {
                        cache_clear_observed,
                        ..
                    },
                    TrackedCommandObservation::CachePause(paused_for_cache),
                ) => {
                    *cache_clear_observed = !paused_for_cache;
                }
                (
                    TrackedCommandKind::Play {
                        restart_sequence_baseline,
                        restart_observed,
                        ..
                    },
                    TrackedCommandObservation::PlaybackRestart(sequence),
                ) => {
                    *restart_observed = sequence > *restart_sequence_baseline;
                }
                (
                    TrackedCommandKind::Play {
                        intent,
                        position_baseline,
                        restart_observed,
                        forward_advancement_observed,
                        ..
                    },
                    TrackedCommandObservation::Position(position_seconds),
                ) => match position_baseline {
                    Some(baseline) => {
                        if (matches!(intent, PlayerPlayIntent::Resume) || *restart_observed)
                            && position_seconds > *baseline + PLAYBACK_ADVANCEMENT_EPSILON_SECONDS
                        {
                            *forward_advancement_observed = true;
                        }
                    }
                    None => *position_baseline = Some(position_seconds),
                },
                _ => {}
            }
        }
        self.finish_completed_tracked_commands();
    }

    fn supersede_tracked_commands(
        &mut self,
        except: Option<PlayerCommandId>,
        predicate: impl Fn(&TrackedCommandKind) -> bool,
    ) {
        let superseded: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| Some(command.id) != except && predicate(&command.kind))
            .map(|command| command.id)
            .collect();
        for command_id in superseded {
            self.finish_tracked_command(command_id, PlayerCommandResult::Superseded);
        }
    }

    fn fail_tracked_commands_for_generation(
        &mut self,
        media_generation: PlayerMediaGeneration,
        failure: PlayerCommandFailureKind,
    ) {
        let result = PlayerCommandResult::Failed(failure);
        let mut failed = Vec::new();
        for command in &mut self.pending_tracked_commands {
            if command.media_generation != Some(media_generation) {
                continue;
            }
            if command.accepted_at.is_some() {
                failed.push(command.id);
            } else {
                command.deferred_result = Some(result);
            }
        }
        for command_id in failed {
            self.finish_tracked_command(command_id, result);
        }
    }

    fn fail_all_accepted_tracked_commands(&mut self, failure: PlayerCommandFailureKind) {
        let failed: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some())
            .map(|command| command.id)
            .collect();
        for command_id in failed {
            self.finish_tracked_command(command_id, PlayerCommandResult::Failed(failure));
        }
    }

    fn expire_tracked_commands(&mut self) {
        let now = Instant::now();
        let timed_out: Vec<_> = self
            .pending_tracked_commands
            .iter()
            .filter(|command| {
                command.accepted_at.is_some_and(|accepted_at| {
                    now.saturating_duration_since(accepted_at) >= command.kind.timeout()
                })
            })
            .map(|command| command.id)
            .collect();
        for command_id in timed_out {
            self.finish_tracked_command(
                command_id,
                PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
            );
        }
    }

    fn observation_timestamp(&self) -> PlayerObservationTimestamp {
        PlayerObservationTimestamp::from_adapter_start(self.observation_clock_origin.elapsed())
    }

    fn transport_update(&self) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate {
            media_generation: self
                .active_media_generation
                .or(self.pending_load_generation),
            observed_at: Some(self.observation_timestamp()),
            ..PlayerTransportTelemetryUpdate::default()
        };
        update.timeline_kind = Some(self.timeline_kind);
        if self.timeline_kind == PlayerTimelineKind::SlidingLive {
            update.known_live_seekable_window = self.latest_cached_seekable_window;
        }
        update
    }

    fn observation_media_generation(&self) -> Option<PlayerMediaGeneration> {
        self.active_media_generation
            .or(self.pending_load_generation)
    }

    fn transport_update_for(
        &self,
        generation: PlayerMediaGeneration,
    ) -> PlayerTransportTelemetryUpdate {
        let mut update =
            PlayerTransportTelemetryUpdate::new(generation, self.observation_timestamp());
        if self.observation_media_generation() == Some(generation) {
            update.timeline_kind = Some(self.timeline_kind);
            if self.timeline_kind == PlayerTimelineKind::SlidingLive {
                update.known_live_seekable_window = self.latest_cached_seekable_window;
            }
        }
        update
    }

    fn queue_transport_telemetry_update(&mut self, mut update: PlayerTransportTelemetryUpdate) {
        if update.media_generation.is_none() {
            update.media_generation = self
                .active_media_generation
                .or(self.pending_load_generation);
        }
        if update.observed_at.is_none() {
            update.observed_at = Some(self.observation_timestamp());
        }
        if update.paused_for_cache == Some(true) {
            for pending in self.pending_transport_telemetry_updates.iter_mut().rev() {
                if pending.media_generation != update.media_generation {
                    break;
                }
                if pending.logical_pause == Some(true) {
                    pending.logical_pause = None;
                }
                if pending.phase == Some(PlayerTransportPhase::ReadyPaused)
                    && update.phase.is_some()
                {
                    pending.phase = update.phase;
                }
                if pending.playback_restart_sequence.is_some() {
                    break;
                }
            }
        }

        let update_has_cache_metrics = update.cache_buffering_percent.is_some()
            || update.buffered_ahead_seconds.is_some()
            || update.buffered_ahead_bytes.is_some()
            || update.input_rate_bytes_per_second.is_some();
        let cache_position_boundary =
            self.pending_transport_telemetry_updates
                .back()
                .is_some_and(|pending| {
                    let pending_has_cache_metrics = pending.cache_buffering_percent.is_some()
                        || pending.buffered_ahead_seconds.is_some()
                        || pending.buffered_ahead_bytes.is_some()
                        || pending.input_rate_bytes_per_second.is_some();
                    (update.position_seconds.is_some()
                        && !update_has_cache_metrics
                        && pending_has_cache_metrics)
                        || (update_has_cache_metrics
                            && update.position_seconds.is_none()
                            && pending.position_seconds.is_some())
                });
        let lifecycle_boundary = cache_position_boundary
            || self
                .pending_transport_telemetry_updates
                .back()
                .is_none_or(|pending| {
                    pending.media_generation != update.media_generation
                        || update.playback_restart_sequence.is_some()
                        || update.error_kind.is_some()
                        || update.eof_reached == Some(true)
                        || update
                            .phase
                            .is_some_and(|phase| pending.phase != Some(phase))
                });
        if !lifecycle_boundary
            && let Some(pending) = self.pending_transport_telemetry_updates.back_mut()
        {
            pending.merge_from(update);
            return;
        }

        if self.pending_transport_telemetry_updates.len() >= MAX_PENDING_TRANSPORT_TELEMETRY_UPDATES
        {
            self.pending_transport_telemetry_updates.pop_front();
        }
        self.pending_transport_telemetry_updates.push_back(update);
    }

    fn begin_seek_cache_evidence_epoch(&mut self) {
        let generation = self.observation_media_generation();
        self.cache_buffering_percent = None;
        self.observed_state.cache_buffering_percent = None;
        for pending in &mut self.pending_transport_telemetry_updates {
            if pending.media_generation == generation {
                pending.cache_buffering_percent = None;
                pending.buffered_ahead_seconds = None;
                pending.buffered_ahead_bytes = None;
                pending.input_rate_bytes_per_second = None;
            }
        }
    }

    fn set_transport_phase(&mut self, phase: PlayerTransportPhase) {
        self.transport_phase = phase;
        let mut update = self.transport_update();
        update.phase = Some(phase);
        self.queue_transport_telemetry_update(update);
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Phase(phase),
        );
    }

    fn inferred_transport_phase(&self) -> PlayerTransportPhase {
        if self.active_media_generation.is_none() && self.pending_load_generation.is_none() {
            return PlayerTransportPhase::Empty;
        }
        if self.observed_state.eof_reached == Some(true) {
            return PlayerTransportPhase::Ended;
        }
        if self.observed_state.seeking == Some(true) {
            return PlayerTransportPhase::Seeking;
        }
        if self.observed_state.paused_for_cache == Some(true) {
            return if self.active_generation_has_restarted {
                PlayerTransportPhase::Rebuffering
            } else {
                PlayerTransportPhase::Prebuffering
            };
        }
        if !self.active_file_loaded {
            return if self.active_media_generation.is_some() {
                PlayerTransportPhase::Loading
            } else {
                PlayerTransportPhase::Empty
            };
        }
        if self.observed_state.logical_pause == Some(true) {
            return PlayerTransportPhase::ReadyPaused;
        }
        if self.observed_state.core_idle == Some(true) {
            return if self.active_generation_has_restarted {
                PlayerTransportPhase::Rebuffering
            } else {
                PlayerTransportPhase::Prebuffering
            };
        }
        if self.observed_state.core_idle == Some(false) || self.active_generation_has_restarted {
            return PlayerTransportPhase::Playing;
        }
        PlayerTransportPhase::Prebuffering
    }

    fn refresh_inferred_transport_phase(&mut self) {
        let phase = self.inferred_transport_phase();
        if phase != self.transport_phase {
            self.set_transport_phase(phase);
        }
    }

    fn cache_state_telemetry_update(&mut self, data: &Value) -> PlayerTransportTelemetryUpdate {
        let mut update = self.transport_update();
        let Some(cache_state) = data.as_object() else {
            return update;
        };

        update.seekable_ranges = cache_state
            .get("seekable-ranges")
            .and_then(Value::as_array)
            .map(|ranges| {
                ranges
                    .iter()
                    .filter_map(|range| {
                        let start_seconds = range.get("start")?.as_f64()?;
                        let end_seconds = range.get("end")?.as_f64()?;
                        (start_seconds.is_finite()
                            && end_seconds.is_finite()
                            && start_seconds <= end_seconds)
                            .then_some(PlayerSeekableRange::new(start_seconds, end_seconds))
                    })
                    .collect()
            });
        if let Some(ranges) = update.seekable_ranges.as_deref() {
            self.latest_cached_seekable_window = ranges
                .iter()
                .copied()
                .filter(|range| {
                    range.start_seconds.is_finite()
                        && range.end_seconds.is_finite()
                        && range.end_seconds > range.start_seconds
                })
                .max_by(|left, right| left.end_seconds.total_cmp(&right.end_seconds));
        }
        update.timeline_kind = Some(self.timeline_kind);
        if self.timeline_kind == PlayerTimelineKind::SlidingLive {
            update.known_live_seekable_window = self.latest_cached_seekable_window;
        }
        update.buffered_ahead_seconds = cache_state
            .get("cache-duration")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        update.buffered_ahead_bytes = cache_state
            .get("fw-bytes")
            .and_then(Self::nonnegative_u64_from_json);
        update.input_rate_bytes_per_second = cache_state
            .get("raw-input-rate")
            .and_then(Self::nonnegative_u64_from_json);
        update
    }

    fn refresh_timeline_kind_from_metadata(&mut self) -> bool {
        let previous = self.timeline_kind;
        let Some(generation) = self.active_media_generation else {
            return false;
        };
        if !self.active_file_loaded || self.path_metadata_generation != Some(generation) {
            return false;
        }
        let Some(path) = self.observed_state.path.as_deref() else {
            return false;
        };
        self.timeline_kind = if !uses_network_media_options(path) {
            PlayerTimelineKind::Vod
        } else if self.ytdl_is_live && self.ytdl_is_live_metadata_generation == Some(generation) {
            // mpv 0.39+ publishes yt-dlp's per-file live flag as metadata.
            // Older stock mpv reaches this branch only through Sorotte's
            // generation-bound external probe. Only positive evidence bound
            // to this load is sufficient for a sliding timeline.
            PlayerTimelineKind::SlidingLive
        } else if self.duration_metadata_generation != Some(generation) {
            return false;
        } else if self.observed_state.duration_seconds.is_some() {
            PlayerTimelineKind::Vod
        } else {
            // mpv's cache ranges are local cache state, not a source/DVR
            // window. A durationless network source remains explicitly
            // Unknown unless an upstream integration supplies positive live
            // timeline evidence; guessing here can turn valid VOD seeks into
            // destructive live-edge clamps.
            PlayerTimelineKind::Unknown
        };
        previous != self.timeline_kind
    }

    fn reset_timeline_metadata(&mut self) {
        self.timeline_kind = PlayerTimelineKind::Unknown;
        self.ytdl_is_live = false;
        self.ytdl_is_live_metadata_generation = None;
        self.latest_cached_seekable_window = None;
        self.path_metadata_generation = None;
        self.duration_metadata_generation = None;
        self.ytdl_live_probe_identity = None;
        self.pending_ytdl_live_probe = None;
    }

    fn nonnegative_u64_from_json(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_i64()?.try_into().ok())
    }

    fn metadata_boolean_is_true(value: &Value) -> bool {
        value.as_bool() == Some(true)
            || value
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    fn full_metadata_ytdl_is_live(value: Option<&Value>) -> bool {
        value
            .and_then(Value::as_object)
            .and_then(|metadata| {
                metadata.iter().find_map(|(key, value)| {
                    key.eq_ignore_ascii_case("ytdl_is_live").then_some(value)
                })
            })
            .is_some_and(Self::metadata_boolean_is_true)
    }

    fn observe_ytdl_is_live_for_current_generation(&mut self, is_live: bool) {
        let Some(generation) = self.active_media_generation else {
            return;
        };
        if self.ytdl_is_live_metadata_generation != Some(generation) {
            self.ytdl_is_live = false;
            self.ytdl_is_live_metadata_generation = Some(generation);
        }
        // Once observed, positive per-file live evidence remains
        // authoritative for this generation. mpv can briefly report either
        // the full metadata map or its subproperty as unavailable during
        // demuxer changes; that must not turn a sliding timeline back into
        // VOD.
        self.ytdl_is_live |= is_live;
        if is_live {
            // A patched legacy hook can expose the native tag even though its
            // version number selected the compatibility probe. Positive
            // native evidence makes that duplicate process unnecessary.
            self.pending_ytdl_live_probe = None;
        }
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
    }

    fn maybe_start_ytdl_live_probe(
        &mut self,
        media_generation: PlayerMediaGeneration,
        target: &str,
    ) {
        if YtdlLiveMetadataCapability::from_mpv_version(self.mpv_version)
            != YtdlLiveMetadataCapability::ExternalProbeRequired
            || self.active_media_generation != Some(media_generation)
            || !self.active_file_loaded
            || self.path_metadata_generation != Some(media_generation)
            || self.duration_metadata_generation != Some(media_generation)
            || self.observed_state.duration_seconds.is_some()
            || self.timeline_kind != PlayerTimelineKind::Unknown
            || self.ytdl_is_live
            || self.ytdl_live_probe_identity.is_some()
        {
            return;
        }

        let target = target.trim().to_owned();
        let Some(execution_target) =
            youtube_live_probe_execution_target(&target).map(str::to_owned)
        else {
            return;
        };
        self.ytdl_live_probe_identity = Some((media_generation, target.clone()));
        self.pending_ytdl_live_probe = Some(spawn_ytdl_live_probe(
            self.ytdl_live_probe_executable.clone(),
            self.ytdl_live_probe_path_prefixes.clone(),
            media_generation,
            target,
            execution_target,
            YTDL_LIVE_PROBE_TIMEOUT,
        ));
    }

    fn poll_ytdl_live_probe_completion(&mut self) {
        let completion = match self.pending_ytdl_live_probe.as_ref().map(|pending| {
            pending
                .completion_rx
                .lock()
                .map_err(|_| TryRecvError::Disconnected)?
                .try_recv()
        }) {
            Some(Ok(completion)) => completion,
            Some(Err(TryRecvError::Empty)) | None => return,
            Some(Err(TryRecvError::Disconnected)) => {
                self.pending_ytdl_live_probe = None;
                return;
            }
        };
        let Some(pending) = self.pending_ytdl_live_probe.take() else {
            return;
        };
        if pending.media_generation != completion.media_generation
            || pending.target != completion.target
            || self.active_media_generation != Some(completion.media_generation)
            || self.ytdl_live_probe_identity.as_ref()
                != Some(&(completion.media_generation, completion.target.clone()))
        {
            return;
        }

        if completion.outcome == YtdlLiveProbeOutcome::IsLive(true) {
            self.observe_ytdl_is_live_for_current_generation(true);
        }
    }

    fn chat_input_polling_enabled(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_options_applied
            && self.legacy_syncplay_ui_settings.chat_input_enabled
    }

    fn poll_ipc_events_for_chat_input_if_enabled(&mut self) {
        if !self.chat_input_polling_enabled() {
            return;
        }

        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };

        let _ = ipc_client.get_property(MPV_PROPERTY_PAUSE);
        self.drain_ipc_events_if_attached();
    }

    fn drain_ipc_events_if_attached(&mut self) {
        let pending_events = match self.ipc_client.as_mut() {
            Some(ipc_client) => ipc_client.take_pending_events(),
            None => return,
        };
        for event in pending_events {
            self.handle_ipc_event(&event);
        }
    }

    fn observe_unhealthy_ipc_transport(&mut self) {
        let disconnected = self
            .ipc_client
            .as_ref()
            .is_some_and(|ipc_client| !ipc_client.is_healthy());
        if !disconnected
            || matches!(
                self.transport_phase,
                PlayerTransportPhase::Empty
                    | PlayerTransportPhase::Ended
                    | PlayerTransportPhase::Failed
            )
            || (!self.active_file_loaded && self.pending_load_generation.is_none())
        {
            return;
        }
        let Some(generation) = self.observation_media_generation() else {
            return;
        };
        self.transport_phase = PlayerTransportPhase::Failed;
        self.active_file_loaded = false;
        let update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Failed);
        self.queue_transport_telemetry_update(update);
    }

    fn handle_ipc_event(&mut self, event: &Value) {
        let Some(event_name) = event.get("event").and_then(Value::as_str) else {
            return;
        };

        match event_name {
            MPV_EVENT_START_FILE => {
                self.handle_start_file_event(event);
                return;
            }
            MPV_EVENT_FILE_LOADED => {
                self.handle_file_loaded_event();
                return;
            }
            MPV_EVENT_SEEK => {
                self.handle_seek_event();
                return;
            }
            MPV_EVENT_PLAYBACK_RESTART => {
                self.handle_playback_restart_event();
                return;
            }
            MPV_EVENT_END_FILE => {
                self.handle_end_file_event(event);
                return;
            }
            MPV_EVENT_PROPERTY_CHANGE => {}
            MPV_EVENT_CLIENT_MESSAGE => {
                self.handle_client_message_event(event);
                return;
            }
            _ => return,
        }

        let Some(property_name) = event.get("name").and_then(Value::as_str) else {
            return;
        };
        let data = event.get("data");

        let file_metadata_changed = match property_name {
            MPV_PROPERTY_PATH => {
                let next_path = data.and_then(Value::as_str).map(ToOwned::to_owned);
                if next_path.is_some() && self.active_media_generation.is_none() {
                    let generation = self
                        .pending_load_generation
                        .unwrap_or_else(|| self.allocate_media_generation());
                    self.active_media_generation = Some(generation);
                    self.active_file_loaded = true;
                    self.active_generation_has_restarted = false;
                    self.transport_phase = PlayerTransportPhase::Prebuffering;
                    let update = self
                        .transport_update_for(generation)
                        .with_phase(PlayerTransportPhase::Prebuffering);
                    self.queue_transport_telemetry_update(update);
                }
                self.current_path = next_path.clone();
                self.observed_state.path = next_path.clone();
                self.path_metadata_generation = self.active_media_generation;
                if let (Some(generation), Some(target)) =
                    (self.active_media_generation, next_path.as_deref())
                {
                    self.maybe_start_ytdl_live_probe(generation, target);
                }
                true
            }
            MPV_PROPERTY_DURATION => {
                self.observed_state.duration_seconds = data.and_then(Value::as_f64);
                self.duration_metadata_generation = self.active_media_generation;
                true
            }
            MPV_PROPERTY_FILE_SIZE => {
                self.observed_state.size_bytes = data
                    .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()));
                true
            }
            MPV_PROPERTY_PAUSE => {
                if let Some(paused) = data.and_then(Value::as_bool) {
                    self.paused = paused;
                    self.observed_state.paused = Some(paused);
                    if !paused {
                        self.logical_pause_explicit = false;
                    } else if matches!(
                        self.transport_phase,
                        PlayerTransportPhase::Empty
                            | PlayerTransportPhase::Loading
                            | PlayerTransportPhase::ReadyPaused
                    ) || self
                        .pending_tracked_commands
                        .iter()
                        .any(|command| matches!(command.kind, TrackedCommandKind::Pause { .. }))
                    {
                        self.logical_pause_explicit = true;
                    }
                    let logical_pause = (!paused
                        || self.observed_state.paused_for_cache != Some(true))
                    .then_some(paused);
                    self.observed_state.logical_pause = logical_pause;
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_paused(paused),
                    );
                    if let Some(logical_pause) = logical_pause {
                        let update = self.transport_update().with_logical_pause(logical_pause);
                        self.queue_transport_telemetry_update(update);
                        self.observe_tracked_commands(
                            self.observation_media_generation(),
                            TrackedCommandObservation::LogicalPause(logical_pause),
                        );
                    }
                    self.refresh_inferred_transport_phase();
                } else {
                    self.observed_state.paused = None;
                    self.observed_state.logical_pause = None;
                }
                false
            }
            MPV_PROPERTY_TIME_POS => {
                if let Some(position_seconds) = data.and_then(Value::as_f64) {
                    self.position_seconds = position_seconds;
                    self.observed_state.position_seconds = Some(position_seconds);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_position_seconds(position_seconds),
                    );
                    let update = self
                        .transport_update()
                        .with_position_seconds(position_seconds);
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Position(position_seconds),
                    );
                } else {
                    self.observed_state.position_seconds = None;
                }
                false
            }
            MPV_PROPERTY_SPEED => {
                if let Some(speed) = data.and_then(Value::as_f64) {
                    self.playback_rate = speed;
                    self.observed_state.playback_rate = Some(speed);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_playback_rate(speed),
                    );
                    let mut update = self.transport_update();
                    update.playback_rate = Some(speed);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.playback_rate = None;
                }
                false
            }
            MPV_PROPERTY_PAUSED_FOR_CACHE => {
                if let Some(paused_for_cache) = data.and_then(Value::as_bool) {
                    self.paused_for_cache = paused_for_cache;
                    self.observed_state.paused_for_cache = Some(paused_for_cache);
                    let logical_pause = match self.observed_state.paused {
                        Some(true) if paused_for_cache => None,
                        Some(true) if self.logical_pause_explicit => Some(true),
                        Some(true) => None,
                        paused => paused,
                    };
                    self.observed_state.logical_pause = logical_pause;
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_paused_for_cache(paused_for_cache),
                    );
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.paused_for_cache = Some(paused_for_cache);
                    update.logical_pause = logical_pause;
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::CachePause(paused_for_cache),
                    );
                    if let Some(logical_pause) = logical_pause {
                        self.observe_tracked_commands(
                            self.observation_media_generation(),
                            TrackedCommandObservation::LogicalPause(logical_pause),
                        );
                    }
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.paused_for_cache = None;
                }
                false
            }
            MPV_PROPERTY_CACHE_BUFFERING_STATE => {
                if let Some(cache_buffering_percent) = data.and_then(Value::as_f64) {
                    self.cache_buffering_percent = Some(cache_buffering_percent);
                    self.observed_state.cache_buffering_percent = Some(cache_buffering_percent);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_cache_buffering_percent(cache_buffering_percent),
                    );
                    let mut update = self.transport_update();
                    update.cache_buffering_percent = Some(cache_buffering_percent);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.cache_buffering_percent = None;
                    self.observed_state.cache_buffering_percent = None;
                }
                false
            }
            MPV_PROPERTY_SEEKING => {
                if let Some(seeking) = data.and_then(Value::as_bool) {
                    self.observed_state.seeking = Some(seeking);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.seeking = Some(seeking);
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Seeking(seeking),
                    );
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.seeking = None;
                }
                false
            }
            MPV_PROPERTY_SEEKABLE => {
                if let Some(seekable) = data.and_then(Value::as_bool) {
                    self.observed_state.seekable = Some(seekable);
                    let mut update = self.transport_update();
                    update.seekable = Some(seekable);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.seekable = None;
                }
                false
            }
            MPV_PROPERTY_CORE_IDLE => {
                if let Some(core_idle) = data.and_then(Value::as_bool) {
                    self.observed_state.core_idle = Some(core_idle);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.core_idle = Some(core_idle);
                    self.queue_transport_telemetry_update(update);
                    self.observe_tracked_commands(
                        self.observation_media_generation(),
                        TrackedCommandObservation::Phase(phase),
                    );
                } else {
                    self.observed_state.core_idle = None;
                }
                false
            }
            MPV_PROPERTY_DEMUXER_CACHE_STATE => {
                let update = self.cache_state_telemetry_update(data.unwrap_or(&Value::Null));
                self.queue_transport_telemetry_update(update);
                false
            }
            MPV_PROPERTY_YTDL_IS_LIVE => {
                self.observe_ytdl_is_live_for_current_generation(
                    data.is_some_and(Self::metadata_boolean_is_true),
                );
                false
            }
            MPV_PROPERTY_METADATA => {
                self.observe_ytdl_is_live_for_current_generation(Self::full_metadata_ytdl_is_live(
                    data,
                ));
                false
            }
            MPV_PROPERTY_DEMUXER_CACHE_IDLE => {
                if let Some(demuxer_cache_idle) = data.and_then(Value::as_bool) {
                    self.observed_state.demuxer_cache_idle = Some(demuxer_cache_idle);
                    let mut update = self.transport_update();
                    update.demuxer_cache_idle = Some(demuxer_cache_idle);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.demuxer_cache_idle = None;
                }
                false
            }
            MPV_PROPERTY_EOF_REACHED => {
                if let Some(eof_reached) = data.and_then(Value::as_bool) {
                    self.observed_state.eof_reached = Some(eof_reached);
                    let phase = self.inferred_transport_phase();
                    self.transport_phase = phase;
                    let mut update = self.transport_update().with_phase(phase);
                    update.eof_reached = Some(eof_reached);
                    self.queue_transport_telemetry_update(update);
                } else {
                    self.observed_state.eof_reached = None;
                }
                false
            }
            _ => false,
        };

        if file_metadata_changed {
            if self.refresh_timeline_kind_from_metadata() {
                let update = self.transport_update();
                self.queue_transport_telemetry_update(update);
            }
            let probe_target = self
                .pending_load_request
                .clone()
                .or_else(|| self.current_path.clone());
            if let (Some(generation), Some(target)) =
                (self.active_media_generation, probe_target.as_deref())
            {
                self.maybe_start_ytdl_live_probe(generation, target);
            }
            self.maybe_emit_local_file_update_from_observed_state();
        }
    }

    fn handle_start_file_event(&mut self, event: &Value) {
        // `pause`, `speed`, and `core-idle` are player/core properties rather
        // than file metadata. mpv does not necessarily emit another property
        // change when an already-paused player begins a new file, so retain
        // their last observations across the media-generation boundary.
        let retained_paused = self.observed_state.paused;
        let retained_logical_pause = self.observed_state.logical_pause;
        let retained_playback_rate = self.observed_state.playback_rate;
        let retained_core_idle = self.observed_state.core_idle;
        let requested_probe_target = self.pending_load_request.clone();
        let playlist_entry_id = event.get("playlist_entry_id").and_then(Value::as_u64);
        let generation = playlist_entry_id
            .and_then(|entry_id| self.playlist_entry_generations.get(&entry_id).copied())
            .or(self.pending_load_generation)
            .unwrap_or_else(|| self.allocate_media_generation());

        if let Some(playlist_entry_id) = playlist_entry_id {
            self.playlist_entry_generations
                .insert(playlist_entry_id, generation);
        }
        self.active_playlist_entry_id = playlist_entry_id;
        self.active_media_generation = Some(generation);
        self.active_file_loaded = false;
        self.active_generation_has_restarted = false;
        self.reset_timeline_metadata();
        self.current_path = None;
        self.paused_for_cache = false;
        self.cache_buffering_percent = None;
        self.observed_state.paused = retained_paused;
        self.observed_state.logical_pause = retained_logical_pause;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.observed_state.position_seconds = None;
        self.observed_state.playback_rate = retained_playback_rate;
        self.observed_state.paused_for_cache = None;
        self.observed_state.cache_buffering_percent = None;
        self.observed_state.seeking = None;
        self.observed_state.seekable = None;
        self.observed_state.core_idle = retained_core_idle;
        self.observed_state.demuxer_cache_idle = None;
        self.observed_state.eof_reached = Some(false);
        self.transport_phase = PlayerTransportPhase::Loading;

        let mut update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading);
        update.logical_pause = retained_logical_pause;
        update.playback_rate = retained_playback_rate;
        update.core_idle = retained_core_idle;
        update.eof_reached = Some(false);
        self.queue_transport_telemetry_update(update);
        if let Some(target) = requested_probe_target.as_deref() {
            self.maybe_start_ytdl_live_probe(generation, target);
        }
    }

    fn handle_seek_event(&mut self) {
        self.observed_state.seeking = Some(true);
        self.transport_phase = PlayerTransportPhase::Seeking;
        let mut update = self
            .transport_update()
            .with_phase(PlayerTransportPhase::Seeking);
        update.seeking = Some(true);
        self.queue_transport_telemetry_update(update);
        self.observe_tracked_commands(
            self.observation_media_generation(),
            TrackedCommandObservation::Seeking(true),
        );
    }

    fn handle_playback_restart_event(&mut self) {
        if self.active_media_generation.is_none() {
            self.active_media_generation = self
                .pending_load_generation
                .or_else(|| Some(self.allocate_media_generation()));
        }
        self.active_file_loaded = true;
        self.active_generation_has_restarted = true;
        self.playback_restart_sequence = self.playback_restart_sequence.wrapping_add(1).max(1);
        self.observed_state.seeking = Some(false);
        self.observed_state.eof_reached = Some(false);
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;

        let mut update = self.transport_update().with_phase(phase);
        update.seeking = Some(false);
        update.eof_reached = Some(false);
        update.playback_restart_sequence = Some(self.playback_restart_sequence);
        self.queue_transport_telemetry_update(update);
        let media_generation = self.observation_media_generation();
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Seeking(false));
        self.observe_tracked_commands(
            media_generation,
            TrackedCommandObservation::PlaybackRestart(self.playback_restart_sequence),
        );
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Phase(phase));
    }

    fn handle_file_loaded_event(&mut self) {
        if self.active_media_generation.is_none() {
            self.active_media_generation = self
                .pending_load_generation
                .or_else(|| Some(self.allocate_media_generation()));
        }
        self.active_file_loaded = true;
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        let initial_probe_target = self
            .pending_load_request
            .clone()
            .or_else(|| self.current_path.clone());
        if let (Some(generation), Some(target)) = (
            self.active_media_generation,
            initial_probe_target.as_deref(),
        ) {
            self.maybe_start_ytdl_live_probe(generation, target);
        }
        let phase = self.inferred_transport_phase();
        self.transport_phase = phase;
        let update = self.transport_update().with_phase(phase);
        self.queue_transport_telemetry_update(update);
        let media_generation = self.observation_media_generation();
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::FileLoaded);
        self.observe_tracked_commands(media_generation, TrackedCommandObservation::Phase(phase));

        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };
        self.pending_load_generation = None;

        let polled_update = self
            .ipc_client
            .as_mut()
            .and_then(|ipc_client| Self::poll_local_file_update_from_mpv(ipc_client).ok())
            .flatten();
        let metadata_is_current = polled_update.is_some();
        let loaded_update =
            polled_update.unwrap_or_else(|| Self::local_file_update_for_path(&requested_target));
        self.current_path = loaded_update.path.clone();
        self.observed_state.path = loaded_update.path.clone();
        self.observed_state.duration_seconds = loaded_update.duration_seconds;
        self.observed_state.size_bytes = loaded_update.size_bytes;
        if metadata_is_current {
            self.path_metadata_generation = self.active_media_generation;
            self.duration_metadata_generation = self.active_media_generation;
        }
        if self.refresh_timeline_kind_from_metadata() {
            let update = self.transport_update();
            self.queue_transport_telemetry_update(update);
        }
        if let Some(generation) = self.active_media_generation {
            self.maybe_start_ytdl_live_probe(generation, &requested_target);
        }
        if Self::local_file_update_ready_for_sync(&loaded_update) {
            self.record_local_file_update_if_changed(loaded_update.clone());
        }
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(
                requested_target,
                loaded_update.path.clone(),
            ));
    }

    fn handle_end_file_event(&mut self, event: &Value) {
        let reason = event
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let playlist_entry_id = event.get("playlist_entry_id").and_then(Value::as_u64);
        let generation = playlist_entry_id
            .and_then(|entry_id| self.playlist_entry_generations.remove(&entry_id))
            .or_else(|| {
                (playlist_entry_id.is_none() || self.active_playlist_entry_id == playlist_entry_id)
                    .then_some(self.active_media_generation)
                    .flatten()
            })
            .or(self.pending_load_generation);
        let message = (reason == MPV_END_FILE_REASON_ERROR).then(|| {
            event
                .get("file_error")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    event.get("error").and_then(|value| match value {
                        Value::String(message) => Some(message.trim().to_owned()),
                        Value::Number(number) => Some(format!("mpv error code {number}")),
                        _ => None,
                    })
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "mpv failed to load the requested media.".to_owned())
        });
        let error_kind = message
            .as_deref()
            .map(Self::media_load_failure_kind_from_message);
        let phase = if error_kind.is_some() {
            PlayerTransportPhase::Failed
        } else {
            PlayerTransportPhase::Ended
        };

        if let Some(generation) = generation {
            let mut update = self.transport_update_for(generation).with_phase(phase);
            update.eof_reached = Some(true);
            update.error_kind = error_kind;
            self.queue_transport_telemetry_update(update);
            self.fail_tracked_commands_for_generation(
                generation,
                PlayerCommandFailureKind::MediaEnded,
            );
        }

        let affects_current_generation = generation.is_some()
            && (generation == self.pending_load_generation
                || (self.pending_load_generation.is_none()
                    && generation == self.active_media_generation));
        if affects_current_generation {
            self.transport_phase = phase;
            self.active_file_loaded = false;
            self.reset_timeline_metadata();
            self.observed_state.eof_reached = Some(true);
            if self.active_playlist_entry_id == playlist_entry_id {
                self.active_playlist_entry_id = None;
            }
        }

        if reason != MPV_END_FILE_REASON_ERROR
            || generation.is_some_and(|generation| {
                self.pending_load_generation.is_some()
                    && self.pending_load_generation != Some(generation)
            })
        {
            return;
        }

        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };
        self.pending_load_generation = None;
        let message = message.expect("error end-file events should have a fallback message");
        self.current_path = None;
        self.pending_local_file_update = None;
        self.last_polled_local_file_update = None;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.reset_timeline_metadata();
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::failure(
                requested_target,
                None,
                error_kind.unwrap_or(PlayerMediaLoadFailureKind::Unknown),
                message,
            ));
    }

    fn handle_client_message_event(&mut self, event: &Value) {
        let Some(args) = event.get("args").and_then(Value::as_array) else {
            return;
        };
        let Some(message_name) = args.first().and_then(Value::as_str) else {
            return;
        };
        let payload = args.get(1).and_then(Value::as_str);
        match message_name {
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED => {
                self.handle_legacy_syncplayintf_options_ack(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG => {
                self.handle_legacy_syncplayintf_pong(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED => {
                self.handle_legacy_syncplayintf_lease_expired(payload);
            }
            LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT => {
                self.handle_legacy_syncplayintf_chat_request(payload);
            }
            _ => {}
        }
    }

    fn handle_legacy_syncplayintf_options_ack(&mut self, payload: Option<&str>) {
        let parsed = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok());
        let Some(parsed) = parsed else {
            if self
                .legacy_syncplayintf_pending_options_generation
                .is_some()
            {
                self.legacy_syncplayintf_options_ack_error = Some(
                    "Sorotte syncplayintf returned a malformed settings acknowledgement".to_owned(),
                );
            }
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(pending_generation) = self.legacy_syncplayintf_pending_options_generation else {
            return;
        };
        let Some(generation) = parsed.get("generation").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            self.legacy_syncplayintf_options_ack_error =
                Some("Sorotte syncplayintf acknowledgement omitted a valid generation".to_owned());
            return;
        };
        if generation < pending_generation {
            return;
        }
        if generation > pending_generation {
            self.legacy_syncplayintf_options_ack_error = Some(format!(
                "Sorotte syncplayintf acknowledged unexpected future generation {generation} while waiting for {pending_generation}"
            ));
            return;
        }
        match parsed.get("status").and_then(Value::as_str) {
            Some("applied") => {
                self.legacy_syncplayintf_options_applied = true;
                self.legacy_syncplayintf_pending_options_generation = None;
                self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
                self.legacy_syncplayintf_options_ack_error = None;
                self.legacy_syncplayintf_lease_reacquire_required = false;
                let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                    SorotteBridgeHealth::Ready
                } else {
                    SorotteBridgeHealth::Disabled
                };
                self.set_sorotte_bridge_health(health);
            }
            Some(status @ ("busy" | "rejected")) => {
                let detail = parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("the bridge rejected the settings update");
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf did not apply generation {generation}: {detail}"
                ));
                if status == "busy" {
                    self.legacy_syncplayintf_lease_reacquire_required = true;
                }
            }
            _ => {
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf returned an invalid status for generation {generation}"
                ));
            }
        }
    }

    fn handle_legacy_syncplayintf_pong(&mut self, payload: Option<&str>) {
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL) {
            return;
        }
        let Some(nonce) = parsed.get("nonce").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        if self.legacy_syncplayintf_pending_ping_nonce != Some(nonce) {
            return;
        }
        let Some(bridge_instance_id) = parsed
            .get("bridgeInstanceId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let Some(script_name) = parsed
            .get("scriptName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let bridge_instance_changed = self
            .legacy_syncplayintf_bridge_instance_id
            .as_deref()
            .is_some_and(|current| current != bridge_instance_id);
        if bridge_instance_changed {
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
        }
        self.legacy_syncplayintf_bridge_instance_id = Some(bridge_instance_id.to_owned());
        self.legacy_syncplayintf_script_name = script_name.to_owned();
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
        if bridge_instance_changed {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::Discovery,
                format!(
                    "Sorotte's mpv bridge instance changed to {bridge_instance_id}; reapplying runtime settings"
                ),
                false,
            );
        }
    }

    fn handle_legacy_syncplayintf_lease_expired(&mut self, payload: Option<&str>) {
        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = Some(
            "Sorotte syncplayintf input lease expired; reapplying the current settings".to_owned(),
        );
        self.begin_sorotte_bridge_runtime_recovery(
            SorotteBridgeFailureKind::AcknowledgementTimeout,
            "Sorotte syncplayintf input lease expired; reapplying the current settings",
            false,
        );
    }

    fn handle_legacy_syncplayintf_chat_request(&mut self, payload: Option<&str>) {
        if !self.chat_input_polling_enabled() {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(message) = parsed.get("text").and_then(Value::as_str) else {
            return;
        };
        self.pending_chat_requests.push_back(message.to_owned());
    }

    fn maybe_emit_local_file_update_from_observed_state(&mut self) {
        if self.pending_load_request.is_some() {
            return;
        }
        let Some(path) = self.observed_state.path.as_deref() else {
            return;
        };

        let mut update = Self::local_file_update_for_path(path);
        if let Some(duration_seconds) = self.observed_state.duration_seconds {
            update = update.with_duration_seconds(duration_seconds);
        }
        if let Some(size_bytes) = self.observed_state.size_bytes {
            update = update.with_size_bytes(size_bytes);
        }
        if Self::local_file_update_ready_for_sync(&update) {
            self.record_local_file_update_if_changed(update);
        }
    }

    fn send_ipc_command_if_attached(&mut self, command: Value) -> Result<(), PlayerError> {
        if let Some(ipc_client) = self.ipc_client.as_mut() {
            ipc_client
                .send_command_expect_success(command)
                .map_err(PlayerError::OperationFailed)?;
        } else if !self.simulation_mode {
            return Err(PlayerError::NotConnected);
        }
        self.drain_ipc_events_if_attached();
        Ok(())
    }

    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
        };

        LocalFileUpdate::new(name)
            .with_size_bytes(size_bytes)
            .with_path(path.to_owned())
    }

    fn local_file_update_ready_for_sync(update: &LocalFileUpdate) -> bool {
        match update.path.as_deref() {
            Some(path) if !path.contains("://") => update.duration_seconds.is_some(),
            _ => true,
        }
    }

    fn local_file_update_matches_request(update: &LocalFileUpdate, requested_target: &str) -> bool {
        if requested_target.trim().is_empty() {
            return false;
        }

        if let Some(path) = update.path.as_deref()
            && Self::media_target_matches(path, requested_target)
        {
            return true;
        }

        if Self::media_target_matches(&update.name, requested_target) {
            return true;
        }

        Path::new(requested_target)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|requested_name| Self::media_target_matches(&update.name, requested_name))
    }

    fn media_target_matches(left: &str, right: &str) -> bool {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }

    fn media_load_failure_kind_from_message(message: &str) -> PlayerMediaLoadFailureKind {
        let normalized = message.to_ascii_lowercase();
        if normalized.contains("failed to recognize file format")
            || normalized.contains("unsupported")
        {
            return PlayerMediaLoadFailureKind::FormatUnsupported;
        }
        if (normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno"))
            && (normalized.contains("not found")
                || normalized.contains("not enough permissions")
                || normalized.contains("no such file"))
        {
            return PlayerMediaLoadFailureKind::HelperMissing;
        }
        if normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno")
        {
            return PlayerMediaLoadFailureKind::HelperBroken;
        }
        if normalized.contains("connection")
            || normalized.contains("network")
            || normalized.contains("http")
            || normalized.contains("timed out")
        {
            return PlayerMediaLoadFailureKind::Network;
        }
        if normalized.contains("aborted") || normalized.contains("interrupt") {
            return PlayerMediaLoadFailureKind::LoadAborted;
        }
        PlayerMediaLoadFailureKind::Unknown
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport(transport: impl MpvJsonIpcTransport + 'static) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport_and_registered_observers(
        transport: impl MpvJsonIpcTransport + 'static,
    ) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            observers_registered: true,
            transport_observers_registered: true,
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport_and_ipc_timeout(
        transport: impl MpvJsonIpcTransport + 'static,
        command_timeout: std::time::Duration,
    ) -> Self {
        let mut adapter = Self {
            ipc_client: Some(MpvJsonIpcClient::new_with_command_timeout(
                Box::new(transport),
                command_timeout,
            )),
            ..Self::default()
        };
        adapter.reset_legacy_syncplayintf_attachment_for_new_ipc();
        adapter
    }

    #[cfg(test)]
    pub(crate) fn enable_test_legacy_chat_input(&mut self) {
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_bridge_instance_id = Some("test-bridge".to_owned());
        self.legacy_syncplayintf_owner_id = "test-owner".to_owned();
        self.legacy_syncplayintf_attachment_id = "test-attachment".to_owned();
        self.legacy_syncplayintf_options_applied = true;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = Some(1);
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplay_ui_settings.chat_input_enabled = true;
    }

    #[cfg(test)]
    pub(crate) fn reset_test_legacy_syncplayintf_attachment(&mut self) {
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
    }

    #[cfg(test)]
    pub(crate) fn replace_test_ipc_transport(
        &mut self,
        transport: impl MpvJsonIpcTransport + 'static,
    ) {
        self.release_sorotte_bridge_best_effort();
        self.collect_ipc_connection_events();
        self.simulation_mode = false;
        self.ipc_client = Some(MpvJsonIpcClient::new(Box::new(transport)));
        self.ipc_endpoint = None;
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
        self.observers_registered = false;
        self.transport_observers_registered = false;
        self.loadfile_options_syntax = None;
        self.mpv_version = None;
        self.legacy_syncplay_osd_placement_restore = None;
    }

    #[cfg(test)]
    pub(crate) fn force_test_legacy_syncplayintf_heartbeat_due(&mut self) {
        self.legacy_syncplayintf_last_heartbeat_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL);
        self.maintain_legacy_syncplayintf_lease();
    }

    #[cfg(test)]
    pub(crate) fn force_test_legacy_syncplayintf_discovery_due(&mut self) {
        self.legacy_syncplayintf_last_discovery_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL);
        self.maintain_legacy_syncplayintf_lease();
    }

    #[cfg(test)]
    pub(crate) fn configure_test_bundled_sorotte_bridge_without_retry(
        &mut self,
    ) -> SorotteBridgeHealth {
        self.configure_bundled_sorotte_bridge_inner(Duration::ZERO)
    }

    #[cfg(test)]
    pub(crate) fn set_test_sorotte_bridge_owner_id(&mut self, owner_id: impl Into<String>) {
        self.legacy_syncplayintf_owner_id = owner_id.into();
    }

    #[cfg(test)]
    pub(crate) fn queue_test_pending_chat_request(&mut self, message: impl Into<String>) {
        self.pending_chat_requests.push_back(message.into());
    }
}

#[cfg(test)]
mod timeline_kind_tests {
    use super::*;
    use crate::live_probe::{YtdlLiveProbeCompletion, YtdlLiveProbeOutcome};

    fn loaded_adapter(path: &str, duration_seconds: Option<f64>) -> MpvAdapter {
        let generation = PlayerMediaGeneration::new(41);
        let mut adapter = MpvAdapter {
            active_file_loaded: true,
            active_media_generation: Some(generation),
            next_media_generation: 42,
            current_path: Some(path.to_owned()),
            path_metadata_generation: Some(generation),
            duration_metadata_generation: Some(generation),
            observed_state: MpvObservedState {
                path: Some(path.to_owned()),
                duration_seconds,
                seekable: Some(true),
                ..MpvObservedState::default()
            },
            ..MpvAdapter::default()
        };
        adapter.refresh_timeline_kind_from_metadata();
        adapter
    }

    fn observe_ytdl_is_live(adapter: &mut MpvAdapter, data: Value) {
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_YTDL_IS_LIVE,
            "data": data,
        }));
    }

    fn observe_full_metadata(adapter: &mut MpvAdapter, data: Value) {
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": MPV_PROPERTY_METADATA,
            "data": data,
        }));
    }

    fn install_test_live_probe(
        adapter: &mut MpvAdapter,
        media_generation: PlayerMediaGeneration,
        target: &str,
    ) -> std::sync::mpsc::Sender<YtdlLiveProbeCompletion> {
        let (completion_tx, completion_rx) = std::sync::mpsc::channel();
        adapter.ytdl_live_probe_identity = Some((media_generation, target.to_owned()));
        adapter.pending_ytdl_live_probe = Some(PendingYtdlLiveProbe {
            media_generation,
            target: target.to_owned(),
            completion_rx: std::sync::Mutex::new(completion_rx),
            cancellation: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        completion_tx
    }

    #[test]
    fn youtube_live_metadata_is_positive_sliding_timeline_evidence() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

        observe_ytdl_is_live(&mut adapter, json!("true"));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert_eq!(
            adapter.ytdl_is_live_metadata_generation,
            adapter.active_media_generation
        );
    }

    #[test]
    fn legacy_full_metadata_event_detects_youtube_live_media() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

        observe_full_metadata(
            &mut adapter,
            json!({ "title": "Live channel", "ytdl_is_live": "true" }),
        );

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(adapter.ytdl_is_live);
    }

    #[test]
    fn current_generation_external_probe_promotes_stock_old_mpv_to_sliding_live() {
        let target = "https://www.youtube.com/watch?v=live";
        let mut adapter = loaded_adapter(target, None);
        adapter.mpv_version = Some((0, 34));
        let generation = adapter.active_media_generation.unwrap();
        let completion_tx = install_test_live_probe(&mut adapter, generation, target);
        completion_tx
            .send(YtdlLiveProbeCompletion {
                media_generation: generation,
                target: target.to_owned(),
                outcome: YtdlLiveProbeOutcome::IsLive(true),
            })
            .unwrap();

        adapter.poll_ytdl_live_probe_completion();

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, Some(generation));
        assert!(adapter.pending_ytdl_live_probe.is_none());
        assert!(
            adapter
                .pending_transport_telemetry_updates
                .iter()
                .any(|update| update.timeline_kind == Some(PlayerTimelineKind::SlidingLive))
        );
    }

    #[test]
    fn failed_or_timed_out_external_probe_never_guesses_live_or_vod() {
        for outcome in [YtdlLiveProbeOutcome::Failed, YtdlLiveProbeOutcome::TimedOut] {
            let target = "https://www.youtube.com/watch?v=unknown";
            let mut adapter = loaded_adapter(target, None);
            adapter.mpv_version = Some((0, 34));
            let generation = adapter.active_media_generation.unwrap();
            let completion_tx = install_test_live_probe(&mut adapter, generation, target);
            completion_tx
                .send(YtdlLiveProbeCompletion {
                    media_generation: generation,
                    target: target.to_owned(),
                    outcome,
                })
                .unwrap();

            adapter.poll_ytdl_live_probe_completion();

            assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
            assert!(!adapter.ytdl_is_live);
            assert!(adapter.pending_ytdl_live_probe.is_none());
        }
    }

    #[test]
    fn external_probe_completion_from_prior_start_file_generation_is_inert() {
        let old_target = "https://www.youtube.com/watch?v=old";
        let mut adapter = loaded_adapter(old_target, None);
        adapter.mpv_version = Some((0, 34));
        let old_generation = adapter.active_media_generation.unwrap();
        let completion_tx = install_test_live_probe(&mut adapter, old_generation, old_target);
        let cancellation = adapter
            .pending_ytdl_live_probe
            .as_ref()
            .map(|pending| std::sync::Arc::clone(&pending.cancellation))
            .unwrap();
        completion_tx
            .send(YtdlLiveProbeCompletion {
                media_generation: old_generation,
                target: old_target.to_owned(),
                outcome: YtdlLiveProbeOutcome::IsLive(true),
            })
            .unwrap();

        adapter.handle_start_file_event(&json!({ "playlist_entry_id": 9001 }));
        adapter.poll_ytdl_live_probe_completion();

        assert_ne!(adapter.active_media_generation, Some(old_generation));
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, None);
        assert!(adapter.pending_ytdl_live_probe.is_none());
        assert!(adapter.ytdl_live_probe_identity.is_none());
        assert!(cancellation.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn unknown_or_native_mpv_versions_do_not_launch_the_legacy_probe() {
        let target = "https://www.youtube.com/watch?v=live";
        for version in [None, Some((0, 39)), Some((1, 0))] {
            let mut adapter = loaded_adapter(target, None);
            adapter.mpv_version = version;
            let generation = adapter.active_media_generation.unwrap();

            adapter.maybe_start_ytdl_live_probe(generation, target);

            assert!(adapter.pending_ytdl_live_probe.is_none());
            assert!(adapter.ytdl_live_probe_identity.is_none());
        }
    }

    #[test]
    fn old_mpv_probe_waits_for_a_loaded_durationless_generation_and_skips_vod() {
        let target = "https://www.youtube.com/watch?v=live";
        let mut adapter = loaded_adapter(target, None);
        adapter.mpv_version = Some((0, 34));
        adapter.ytdl_live_probe_executable =
            Some(PathBuf::from("definitely-missing-sorotte-ytdl-live-probe"));
        let generation = adapter.active_media_generation.unwrap();
        adapter.active_file_loaded = false;

        adapter.maybe_start_ytdl_live_probe(generation, target);
        assert!(adapter.pending_ytdl_live_probe.is_none());

        adapter.active_file_loaded = true;
        adapter.duration_metadata_generation = None;
        adapter.maybe_start_ytdl_live_probe(generation, target);
        assert!(adapter.pending_ytdl_live_probe.is_none());

        adapter.duration_metadata_generation = Some(generation);
        adapter.maybe_start_ytdl_live_probe(generation, target);
        assert!(adapter.pending_ytdl_live_probe.is_some());
        adapter.reset_timeline_metadata();

        let mut vod = loaded_adapter(target, Some(120.0));
        vod.mpv_version = Some((0, 34));
        let vod_generation = vod.active_media_generation.unwrap();
        vod.maybe_start_ytdl_live_probe(vod_generation, target);
        assert!(vod.pending_ytdl_live_probe.is_none());
        assert!(vod.ytdl_live_probe_identity.is_none());
    }

    #[test]
    fn absent_or_false_live_metadata_keeps_durationless_network_media_unknown() {
        for data in [Value::Null, json!(false), json!("false")] {
            let mut adapter = loaded_adapter("https://media.invalid/unknown.m3u8", None);

            observe_ytdl_is_live(&mut adapter, data);

            assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
            assert!(!adapter.ytdl_is_live);
        }
    }

    #[test]
    fn positive_live_metadata_is_sticky_for_the_active_generation() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_full_metadata(&mut adapter, json!({ "ytdl_is_live": "true" }));

        observe_ytdl_is_live(&mut adapter, Value::Null);
        observe_ytdl_is_live(&mut adapter, json!("false"));
        observe_full_metadata(&mut adapter, json!({ "title": "metadata refresh" }));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(adapter.ytdl_is_live);

        let mut reverse_order = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut reverse_order, json!("true"));
        observe_full_metadata(&mut reverse_order, json!({ "ytdl_is_live": "false" }));
        assert_eq!(reverse_order.timeline_kind, PlayerTimelineKind::SlidingLive);
        assert!(reverse_order.ytdl_is_live);
    }

    #[test]
    fn finite_duration_network_media_is_vod_without_positive_live_metadata() {
        let mut adapter = loaded_adapter("https://media.invalid/movie.m3u8", Some(120.0));
        observe_ytdl_is_live(&mut adapter, json!("false"));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
    }

    #[test]
    fn local_paths_and_file_urls_are_always_vod() {
        for path in ["C:/media/movie.mkv", "file:///C:/media/movie.mkv"] {
            let mut adapter = loaded_adapter(path, None);
            observe_ytdl_is_live(&mut adapter, json!("true"));

            assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
        }
    }

    #[test]
    fn new_generation_clears_live_evidence_and_rejects_stale_metadata() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));
        let previous_generation = adapter.active_media_generation;
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

        adapter.handle_start_file_event(&json!({ "playlist_entry_id": 42 }));
        let current_generation = adapter.active_media_generation;
        assert_ne!(current_generation, previous_generation);
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, None);

        adapter.active_file_loaded = true;
        adapter.current_path = Some("https://media.invalid/next.m3u8".to_owned());
        adapter.observed_state.path = adapter.current_path.clone();
        adapter.observed_state.duration_seconds = None;
        adapter.path_metadata_generation = current_generation;
        adapter.duration_metadata_generation = current_generation;
        adapter.ytdl_is_live = true;
        adapter.ytdl_is_live_metadata_generation = previous_generation;
        adapter.refresh_timeline_kind_from_metadata();

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
    }

    #[test]
    fn ending_the_active_generation_clears_live_evidence() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));
        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

        adapter.handle_end_file_event(&json!({ "reason": "eof" }));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
        assert_eq!(adapter.ytdl_is_live_metadata_generation, None);
    }

    #[test]
    fn empty_cache_range_snapshot_clears_the_conservative_live_window() {
        let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
        observe_ytdl_is_live(&mut adapter, json!("true"));

        let populated = adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [{ "start": 80.0, "end": 100.0 }],
        }));
        assert_eq!(
            populated.known_live_seekable_window,
            Some(PlayerSeekableRange::new(80.0, 100.0))
        );

        let cleared = adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [],
        }));
        assert_eq!(cleared.seekable_ranges, Some(Vec::new()));
        assert_eq!(cleared.known_live_seekable_window, None);
        assert_eq!(adapter.latest_cached_seekable_window, None);
    }
}
