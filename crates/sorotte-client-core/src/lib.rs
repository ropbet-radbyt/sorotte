use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::Md5;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sorotte_core::SyncDomain;
use sorotte_media_match::{MEDIA_MATCH_FILE_PAYLOAD_KEY, MediaMatchTier};
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
};
use sorotte_protocol::{
    ChatPayload, ControllerAuthPayload, FilePayload, IgnoringOnTheFlyPayload, ListPayload,
    PingPayload, PlaylistIndexPayload, PlaystatePayload, ProtocolError, ProtocolMessage,
    ReadyPayload, RoomRef, SetPayload, StatePayload, canonical_playlist_files_from_change,
    decode_message_line, decode_message_line_items, encode_message_line,
    extract_hello_from_message, playlist_change_with_plex_sidecar,
};

const SEEK_THRESHOLD_SECONDS: f64 = 1.0;
const DEFAULT_REWIND_THRESHOLD_SECONDS: f64 = 4.0;
const DEFAULT_FASTFORWARD_THRESHOLD_SECONDS: f64 = 5.0;
const FASTFORWARD_BEHIND_THRESHOLD_SECONDS: f64 = 1.75;
const FASTFORWARD_EXTRA_SECONDS: f64 = 0.25;
const FASTFORWARD_RESET_THRESHOLD_SECONDS: f64 = 3.0;
const DEFAULT_SLOWDOWN_THRESHOLD_SECONDS: f64 = 1.5;
const SLOWDOWN_RESET_THRESHOLD_SECONDS: f64 = 0.1;
const SLOWDOWN_RATE: f64 = 0.95;
const NORMAL_PLAYBACK_RATE: f64 = 1.0;
const DEFAULT_MAX_RECONNECT_RETRIES: u32 = 999;
const DEFAULT_RECONNECT_BASE_DELAY_SECONDS: f64 = 0.1;
const DEFAULT_RECONNECT_BACKOFF_MAX_EXPONENT: u32 = 5;
const DEFAULT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS: f64 = 2.0;
const DEFAULT_AUTOPLAY_DELAY_SECONDS: f64 = 3.0;
const AUTOPLAY_COUNTDOWN_STEP_SECONDS: f64 = 1.0;
const RECENTLY_ADVANCED_GRACE_SECONDS: f64 = 5.0;
const RECENT_REWIND_READINESS_SUPPRESSION_SECONDS: f64 = 5.0;
const RECENT_REWIND_SEEK_SUPPRESSION_SECONDS: f64 = 1.0;
const RECENT_REWIND_SEEK_IGNORE_POSITION_SECONDS: f64 = 5.0;
const LEGACY_SHOW_DURATION_NOTIFICATION: bool = true;
const LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS: f64 = 2.5;
const LEGACY_CHAT_MAX_MESSAGE_LENGTH: usize = 150;
const LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH: usize = 50;
const LEGACY_FALLBACK_MAX_USERNAME_LENGTH: usize = 16;
const LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH: usize = 35;
const LEGACY_FALLBACK_MAX_FILENAME_LENGTH: usize = 250;
pub const SYNCPLAY_WIRE_VERSION_LEGACY: &str = "1.2.255";
pub const SYNCPLAY_COMPAT_VERSION_LEGACY: &str = "1.7.5";
const LEGACY_CHAT_MIN_VERSION: &str = "1.5.0";
const LEGACY_USER_READY_MIN_VERSION: &str = "1.3.0";
const LEGACY_MANAGED_ROOMS_MIN_VERSION: &str = "1.3.0";
const LEGACY_SHARED_PLAYLIST_MIN_VERSION: &str = "1.4.0";
const LEGACY_SET_OTHERS_READINESS_MIN_VERSION: &str = "1.7.2";
const LEGACY_SHOW_SAME_ROOM_OSD: bool = true;
const LEGACY_SHOW_OSD_WARNINGS: bool = true;
const LEGACY_SHOW_NONCONTROLLER_OSD: bool = false;
const LEGACY_SHOW_DIFFERENT_ROOM_OSD: bool = false;
const LEGACY_ONLY_SWITCH_TO_TRUSTED_DOMAINS: bool = true;
const LEGACY_DEFAULT_TRUSTED_DOMAINS: [&str; 2] = ["youtube.com", "youtu.be"];
const DEFAULT_RECONNECT_STATE_RESTORE_AUTOCORRECT: bool = true;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS: u32 = 1;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS: u32 = 8;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES: u32 = 0;
const ROUND_HALF_EPSILON: f64 = 1e-12;
const PRIVACY_HIDDEN_FILENAME: &str = "**Hidden filename**";
const MUSIC_FORMATS: [&str; 8] = [
    ".mp3", ".m4a", ".m4p", ".wav", ".aiff", ".r", ".ogg", ".flac",
];
pub const AUTOPLAY_TICK_INTERVAL_SECONDS: f64 = AUTOPLAY_COUNTDOWN_STEP_SECONDS;
const LEGACY_PING_MOVING_AVERAGE_WEIGHT: f64 = 0.85;

pub fn legacy_server_password_token(password: &str) -> String {
    format!("{:x}", Md5::digest(password.as_bytes()))
}

mod config;
mod control;
mod notifications;
mod outbox;
mod ping;
mod runtime;
mod session;
mod views;

pub use self::config::{
    ChatConfig, DesyncCorrectionAction, DesyncCorrectionConfig, PrivacyMode,
    ReadinessAutoplayConfig, ReconnectPolicyConfig, ReconnectRetryDecision,
    ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionPolicyMode,
    ReconnectStateRestoreCorrectionStateSnapshot, SessionBehaviorConfig, UnpauseActionMode,
};
pub use self::control::{ClientRuntimeAction, ClientRuntimeControl, QueuedRuntimeControl};
pub use self::notifications::{
    AutoplayCountdownNotification, ChatNotification, ControlledRoomCreationNotification,
    ControllerAuthTransitionNotification, FileDifferenceSummary, ReconnectPlaylistRestoreIntent,
    ReconnectTransitionNotification, UserChangeNotification,
};
pub use self::ping::ClientPingMetricsLegacyCompatible;
pub(crate) use self::ping::unix_wall_clock_time_seconds_legacy_compatible;
pub use self::runtime::ClientRuntime;
pub use self::session::ClientSession;
pub(crate) use self::session::ClientSessionLocalActionSnapshot;
pub use self::views::{
    ClientMediaMatchPeerFileState, ClientUserView, RoomPlaylistView, RoomPlaystateView,
};
