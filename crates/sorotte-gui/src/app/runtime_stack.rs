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
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Map, Value};
use sorotte_client_app::app_boundary::application::{
    ClientApplication, ClientApplicationSettings, ClientCommand, ClientEvent,
};
use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, RoomName, StoredClientSettingsRuntimeSnapshot,
    StreamingQualityDowngradeSuggestion, Username,
    parse_host_and_optional_port_from_host_arg_legacy_compatible,
};
use sorotte_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, ChatNotification, ClientEffect, ClientMediaMatchPeerFileState,
    ClientRuntimeAction, ClientSession, CoordinatorCommandId, CoordinatorPlayerCommand,
    DesyncCorrectionDispatchSnapshot, ExternalPlayerAvailability, LogicalMediaId, MediaLoadIntent,
    MediaLoadPlan, MediaTransportKind, PlaybackBarrierTimeoutAction, PlaybackCoordinationSnapshot,
    PlaybackCoordinatorAction, PlayerCommandCause, PrivacyMode, ProtocolLineLease,
    RoomPlaylistView, RoomPlaystateView, SYNCPLAY_COMPAT_VERSION_LEGACY,
    SYNCPLAY_WIRE_VERSION_LEGACY, legacy_server_password_token,
};
use sorotte_media_match::{MediaMatchTier, MediaMatchWireSignature};
use sorotte_player_api::{
    PlayerCommandId, PlayerError, PlayerMediaGeneration, PlayerPlaybackTelemetryUpdate,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    HelloPayload, ListPayload, ProtocolMessage, SOROTTE_READINESS_RECONNECT_TOKEN,
    decode_message_line_items, encode_message_line,
};

use self::player::GuiNoopClientRuntimePlayer;
#[cfg(not(test))]
use super::remote_services;
use super::shell_state::{
    GuiCommandAvailabilityState, GuiShellAction, GuiTransientNotificationLevel,
    SorotteGuiShellAppState,
};
use super::support::{legacy_chat_input_enabled, system_time_seconds};

pub(super) use self::client_core_adapter::GuiClientCoreChatSessionRuntimeAdapter;
pub(super) use self::player::{
    GuiOwnedPlayer, GuiPlayerLaunchRuntimeState, GuiTestPlayerAdapter,
    local_file_update_for_player_path,
};
pub(super) use self::session_adapter::{
    GuiAttachedPlayerRuntimeAction, GuiLocalPlayerUnpauseDecision,
    GuiPlaylistProtocolDeliveryFence, GuiSessionRoomPlaystate, GuiSessionRuntimeAdapter,
};
#[cfg(test)]
pub(super) use self::transport::GuiTcpSessionTransportDriver;
pub(super) use self::transport::{
    GuiLoopbackSessionTransportDriver, GuiOutboundProtocolDelivery,
    GuiOutboundProtocolDeliveryResult, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
    GuiThreadedTcpSessionTransportDriver,
};
