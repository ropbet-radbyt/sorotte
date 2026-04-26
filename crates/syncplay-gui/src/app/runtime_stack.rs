#[cfg(test)]
mod tests;

mod client_core_adapter;
mod media_search;
mod notifications;
mod player;
mod public_servers;
mod runtime_snapshots;
mod session_adapter;
mod transport;

use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Map, Value};
use syncplay_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsRuntimeSnapshot,
    parse_host_and_optional_port_from_host_arg_legacy_compatible,
};
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, ChatNotification, ClientRuntime, ClientRuntimeAction,
    ClientRuntimeControl, ClientSession, DesyncCorrectionConfig, PrivacyMode, QueuedRuntimeControl,
    ReadinessAutoplayConfig, RoomPlaylistView, RoomPlaystateView, SYNCPLAY_COMPAT_VERSION_LEGACY,
    SYNCPLAY_WIRE_VERSION_LEGACY, SessionBehaviorConfig, legacy_server_password_token,
};
use syncplay_player_api::PlayerPlaybackTelemetryUpdate;
use syncplay_protocol::{
    HelloPayload, ListPayload, ProtocolMessage, decode_message_lines, encode_message_line,
};

use self::player::GuiNoopClientRuntimePlayer;
#[cfg(not(test))]
use super::remote_services;
use super::shell_state::{
    GuiCommandAvailabilityState, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, SyncplayGuiShellAppState,
};
use super::support::{legacy_chat_input_enabled, system_time_seconds};

pub(super) use self::client_core_adapter::GuiClientCoreChatSessionRuntimeAdapter;
pub(super) use self::player::{GuiOwnedPlayer, GuiPlayerLaunchRuntimeState, GuiTestPlayerAdapter};
pub(super) use self::session_adapter::{
    GuiAttachedPlayerRuntimeAction, GuiLocalPlayerUnpauseDecision, GuiSessionRoomPlaystate,
    GuiSessionRuntimeAdapter,
};
pub(super) use self::transport::{
    GuiLoopbackSessionTransportDriver, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
    GuiTcpSessionTransportDriver,
};
