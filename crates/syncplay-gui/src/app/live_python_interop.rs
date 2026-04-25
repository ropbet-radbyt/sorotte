mod flows;
mod projection;
mod runtime_actions;
mod waits;

use std::{fmt, time::Duration};

use syncplay_compat::{InteropError, LegacyPythonPeerChatMessage, LegacyServerPythonPeerHarness};
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_USERNAME: &str = "interop-gui-user";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_USERNAME: &str = "interop-py-peer";
pub(crate) const LIVE_PYTHON_INTEROP_ROOM: &str = "interop-room";
pub(crate) const LIVE_PYTHON_INTEROP_ALT_ROOM: &str = "interop-room-b";
pub(crate) const LIVE_PYTHON_INTEROP_CONTROLLED_ROOM: &str = "+interop-room:447CE7E3548D";
pub(crate) const LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT: &str =
    "+interop-room:447CE7E3548D:AB-123-456";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE: &str = "hello from gui";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE: &str = "hello from python";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE: &str = "hello again from gui";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE: &str = "hello again from python";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE: &str = "gui-playlist-1.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO: &str = "gui-playlist-2.mkv";
#[cfg(test)]
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_ONE: &str = "C:/Media/gui-open-1.mkv";
#[cfg(test)]
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_TWO: &str = "C:/Media/gui-open-2.mkv";
#[cfg(test)]
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE: &str = "gui-open-1.mkv";
#[cfg(test)]
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO: &str = "gui-open-2.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE: &str = "python-playlist-1.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO: &str = "python-playlist-2.mkv";
const LIVE_PYTHON_INTEROP_TIMEOUT: Duration = Duration::from_secs(6);
const LIVE_PYTHON_INTEROP_POLL_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(test)]
const LIVE_PYTHON_INTEROP_KEEPALIVE_OBSERVATION: Duration = Duration::from_secs(13);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LivePythonPeerInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
    pub local_user_ready: bool,
    pub peer_user_ready: bool,
    pub room_switch_observed: bool,
    pub room_rejoin_observed: bool,
    pub peer_disconnect_observed: bool,
    pub peer_reconnect_observed: bool,
    pub gui_playlist: Vec<String>,
    pub gui_playlist_index: Option<usize>,
    pub peer_playlist: Vec<String>,
    pub peer_playlist_index: Option<usize>,
    pub gui_chat_messages: Vec<LegacyPythonPeerChatMessage>,
    pub peer_chat_messages: Vec<LegacyPythonPeerChatMessage>,
    pub widget_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LivePythonPeerControlledRoomInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
    pub local_user_controller: bool,
    pub peer_user_controller: bool,
    pub peer_local_controller: bool,
    pub can_manage_playlist: bool,
    pub widget_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LivePythonPeerDetachedConnectInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
    pub local_user_ready: bool,
    pub peer_user_ready: bool,
    pub widget_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LivePythonPeerSharedPlaylistOpenInteropResult {
    pub room_name: String,
    pub gui_playlist: Vec<String>,
    pub gui_playlist_index: Option<usize>,
    pub peer_playlist: Vec<String>,
    pub peer_playlist_index: Option<usize>,
    pub peer_observed_local_file_name: Option<String>,
    pub widget_count: usize,
}

#[derive(Debug)]
pub(crate) enum LivePythonPeerInteropError {
    Interop(InteropError),
    Gui(String),
}

impl fmt::Display for LivePythonPeerInteropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interop(error) => write!(f, "{error}"),
            Self::Gui(error) => f.write_str(error),
        }
    }
}

impl From<InteropError> for LivePythonPeerInteropError {
    fn from(error: InteropError) -> Self {
        Self::Interop(error)
    }
}

#[cfg(test)]
pub(crate) fn live_python_interop_prerequisites_missing(
    error: &LivePythonPeerInteropError,
) -> bool {
    match error {
        LivePythonPeerInteropError::Interop(error) => {
            syncplay_compat::interop_prerequisites_missing(error)
        }
        LivePythonPeerInteropError::Gui(_) => false,
    }
}

pub(crate) fn run_live_python_peer_connect_flow()
-> Result<LivePythonPeerInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = flows::run_live_python_peer_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

pub(crate) fn run_live_python_peer_controlled_room_flow()
-> Result<LivePythonPeerControlledRoomInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_CONTROLLED_ROOM,
    )?;
    let outcome = flows::run_live_python_peer_controlled_room_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

#[cfg(test)]
pub(crate) fn run_live_python_peer_detached_public_server_connect_flow()
-> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome =
        flows::run_live_python_peer_detached_public_server_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

#[cfg(test)]
pub(crate) fn run_live_python_peer_startup_saved_connect_flow()
-> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = flows::run_live_python_peer_startup_saved_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

#[cfg(test)]
pub(crate) fn run_live_python_peer_shared_playlist_open_flow()
-> Result<LivePythonPeerSharedPlaylistOpenInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = flows::run_live_python_peer_shared_playlist_open_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}
