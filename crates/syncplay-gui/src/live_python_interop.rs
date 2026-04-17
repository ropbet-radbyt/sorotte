use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use syncplay_compat::{
    InteropError, LegacyPythonPeerChatMessage, LegacyPythonPeerSnapshot,
    LegacyServerPythonPeerHarness,
};

use super::{
    GuiOwnedPlayer, GuiPendingCompletionRequest, GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiShellAction,
    GuiShellView, GuiTestPlayerAdapter, StoredClientSettingsMvp, SyncplayGuiShellAppState,
};

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
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_ONE: &str = "C:/Media/gui-open-1.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_TWO: &str = "C:/Media/gui-open-2.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE: &str = "gui-open-1.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO: &str = "gui-open-2.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE: &str = "python-playlist-1.mkv";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO: &str = "python-playlist-2.mkv";
const LIVE_PYTHON_INTEROP_TIMEOUT: Duration = Duration::from_secs(6);
const LIVE_PYTHON_INTEROP_POLL_INTERVAL: Duration = Duration::from_millis(50);
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
pub(crate) struct LivePythonPeerDetachedConnectInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
    pub local_user_ready: bool,
    pub peer_user_ready: bool,
    pub widget_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[allow(dead_code)]
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
    let outcome = run_live_python_peer_connect_flow_with_harness(&mut harness);
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
    let outcome = run_live_python_peer_controlled_room_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

pub(crate) fn run_live_python_peer_detached_public_server_connect_flow()
-> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome =
        run_live_python_peer_detached_public_server_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

pub(crate) fn run_live_python_peer_startup_saved_connect_flow()
-> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = run_live_python_peer_startup_saved_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

pub(crate) fn run_live_python_peer_shared_playlist_open_flow()
-> Result<LivePythonPeerSharedPlaylistOpenInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = run_live_python_peer_shared_playlist_open_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

fn run_live_python_peer_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_ROOM,
            harness.address(),
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_ROOM.to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let startup_deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < startup_deadline {
        pump_and_apply(&mut owner, &handle, &mut state);
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    request_local_room_join(&handle, &mut state, LIVE_PYTHON_INTEROP_ALT_ROOM)?;
    wait_for_projected_room_projection(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_ALT_ROOM,
        false,
    )?;
    let room_switch_observed = true;

    request_local_room_join(&handle, &mut state, LIVE_PYTHON_INTEROP_ROOM)?;
    wait_for_projected_room_projection(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_ROOM,
        true,
    )?;
    let room_rejoin_observed = true;

    request_local_ready(&handle, &mut state, true)?;
    wait_for_projection(&mut owner, &handle, &mut state, true, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        true,
        Duration::from_secs(3),
    )?;

    request_local_ready(&handle, &mut state, false)?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        false,
        Duration::from_secs(3),
    )?;

    harness.set_peer_ready(true)?;
    let _ = harness.wait_for_peer_local_ready(true, Duration::from_secs(3))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, true)?;

    harness.set_peer_ready(false)?;
    let _ = harness.wait_for_peer_local_ready(false, Duration::from_secs(3))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;

    request_local_chat_send(&handle, &mut state, LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE)?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
    )?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;

    harness.send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE)?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
    )?;

    wait_for_playlist_controls(&mut owner, &handle, &mut state)?;

    let first_local_playlist = vec![LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE.to_owned()];
    request_local_playlist_queue(
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE,
        false,
    )?;
    wait_for_projected_playlist(
        &mut owner,
        &handle,
        &mut state,
        &first_local_playlist,
        Some(0),
    )?;
    wait_for_peer_observed_playlist(harness, &first_local_playlist, Duration::from_secs(3))?;
    wait_for_peer_observed_playlist_index(harness, 0, Duration::from_secs(3))?;

    let second_local_playlist = vec![
        LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE.to_owned(),
        LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO.to_owned(),
    ];
    request_local_playlist_queue(
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO,
        false,
    )?;
    wait_for_projected_playlist(
        &mut owner,
        &handle,
        &mut state,
        &second_local_playlist,
        Some(0),
    )?;
    wait_for_peer_observed_playlist(harness, &second_local_playlist, Duration::from_secs(3))?;
    wait_for_peer_observed_playlist_index(harness, 0, Duration::from_secs(3))?;

    request_local_playlist_selection(&handle, &mut state, 1)?;
    wait_for_projected_playlist(
        &mut owner,
        &handle,
        &mut state,
        &second_local_playlist,
        Some(1),
    )?;
    wait_for_peer_observed_playlist_index(harness, 1, Duration::from_secs(3))?;

    let reduced_local_playlist = vec![LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE.to_owned()];
    request_local_playlist_remove_selected(&handle, &mut state)?;
    wait_for_projected_playlist(
        &mut owner,
        &handle,
        &mut state,
        &reduced_local_playlist,
        Some(0),
    )?;
    wait_for_peer_observed_playlist(harness, &reduced_local_playlist, Duration::from_secs(3))?;
    wait_for_peer_observed_playlist_index(harness, 0, Duration::from_secs(3))?;

    let peer_playlist = vec![
        LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
        LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
    ];
    harness.set_peer_playlist(&peer_playlist)?;
    wait_for_peer_observed_playlist(harness, &peer_playlist, Duration::from_secs(3))?;
    wait_for_projected_playlist(&mut owner, &handle, &mut state, &peer_playlist, Some(0))?;

    harness.set_peer_playlist_index(1)?;
    wait_for_peer_observed_playlist_index(harness, 1, Duration::from_secs(3))?;
    wait_for_projected_playlist(&mut owner, &handle, &mut state, &peer_playlist, Some(1))?;

    let mut peer_chat_messages = harness.peer_snapshot()?.chat_messages;
    harness.disconnect_peer()?;
    wait_for_projected_user_absence(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
    )?;
    let peer_disconnect_observed = true;

    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    let peer_reconnect_observed = true;

    request_local_chat_send(
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE,
    )?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE,
    )?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;

    harness.send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE)?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE,
    )?;

    let peer_snapshot = harness.peer_snapshot()?;
    merge_peer_chat_messages(&mut peer_chat_messages, peer_snapshot.chat_messages.clone());
    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        room_switch_observed,
        room_rejoin_observed,
        peer_disconnect_observed,
        peer_reconnect_observed,
        gui_playlist: gui_playlist(&state),
        gui_playlist_index: state.selection.selected_main_window_playlist,
        peer_playlist: peer_snapshot.playlist,
        peer_playlist_index: peer_snapshot.playlist_index,
        gui_chat_messages: state
            .main_window
            .chat
            .iter()
            .map(|row| LegacyPythonPeerChatMessage {
                sender: row.sender.clone(),
                message: row.message.clone(),
            })
            .collect(),
        peer_chat_messages,
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn run_live_python_peer_controlled_room_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerControlledRoomInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT,
            harness.address(),
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT.to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let startup_deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < startup_deadline {
        pump_and_apply(&mut owner, &handle, &mut state);
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
    harness.start_peer_connected()?;
    wait_for_controlled_room_projection(&mut owner, &handle, &mut state)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    wait_for_peer_observed_user_controller(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        true,
        Duration::from_secs(3),
    )?;
    harness.wait_for_peer_local_controller(false, Duration::from_secs(3))?;

    request_remote_user_ready(&handle, &mut state, harness.peer_username(), true)?;
    pump_and_apply(&mut owner, &handle, &mut state);
    harness.wait_for_peer_local_ready(true, Duration::from_secs(3))?;
    wait_for_controlled_room_peer_ready_projection(&mut owner, &handle, &mut state, true)?;

    request_remote_user_ready(&handle, &mut state, harness.peer_username(), false)?;
    pump_and_apply(&mut owner, &handle, &mut state);
    let peer_snapshot = harness.wait_for_peer_local_ready(false, Duration::from_secs(3))?;
    wait_for_controlled_room_peer_ready_projection(&mut owner, &handle, &mut state, false)?;

    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerControlledRoomInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_controller: local_user_controller(&state).unwrap_or(false),
        peer_user_controller: peer_user_controller(&state, harness.peer_username())
            .unwrap_or(false),
        peer_local_controller: peer_snapshot.local_controller.unwrap_or(false),
        can_manage_playlist: state.main_window.playback.can_manage_playlist,
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn run_live_python_peer_detached_public_server_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_ROOM.to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), harness.address().to_owned())]),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    if !state.apply(GuiShellAction::SelectPublicServer(0))
        || !state.apply(GuiShellAction::BeginSelectedPublicServerConnect)
    {
        return Err(LivePythonPeerInteropError::Gui(
            "failed to stage detached public-server connect".to_owned(),
        ));
    }
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    pump_and_apply(&mut owner, &handle, &mut state);
    wait_for_projected_room_projection(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_ROOM,
        false,
    )?;
    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    wait_for_sustained_connection_presence(
        &mut owner,
        &handle,
        &mut state,
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_KEEPALIVE_OBSERVATION,
    )?;

    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerDetachedConnectInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn run_live_python_peer_startup_saved_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("localhost".to_owned()),
        port: Some(harness.port()),
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_ROOM.to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    wait_for_projected_room_projection(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_ROOM,
        false,
    )?;
    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    wait_for_sustained_connection_presence(
        &mut owner,
        &handle,
        &mut state,
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_KEEPALIVE_OBSERVATION,
    )?;

    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerDetachedConnectInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn run_live_python_peer_shared_playlist_open_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerSharedPlaylistOpenInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_ROOM,
            harness.address(),
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_ROOM.to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let startup_deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < startup_deadline {
        pump_and_apply(&mut owner, &handle, &mut state);
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;
    wait_for_playlist_controls(&mut owner, &handle, &mut state)?;
    request_local_ready(&handle, &mut state, true)?;
    wait_for_projection(&mut owner, &handle, &mut state, true, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        true,
        Duration::from_secs(3),
    )?;

    let expected_playlist = vec![
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE.to_owned(),
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO.to_owned(),
    ];
    request_local_shared_playlist_open(
        &handle,
        &[
            LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_ONE,
            LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_PATH_TWO,
        ],
    );
    wait_for_projected_playlist(&mut owner, &handle, &mut state, &expected_playlist, Some(0))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_playlist(harness, &expected_playlist, Duration::from_secs(3))?;
    wait_for_peer_observed_playlist_index(harness, 0, Duration::from_secs(3))?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        false,
        Duration::from_secs(3),
    )?;
    let peer_snapshot = wait_for_peer_observed_user_file_name(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE,
        Duration::from_secs(3),
    )?;

    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerSharedPlaylistOpenInteropResult {
        room_name: state.main_window.room_name.clone(),
        gui_playlist: gui_playlist(&state),
        gui_playlist_index: state.selection.selected_main_window_playlist,
        peer_playlist: peer_snapshot.playlist,
        peer_playlist_index: peer_snapshot.playlist_index,
        peer_observed_local_file_name: peer_snapshot
            .observed_user_file_names
            .get(LIVE_PYTHON_INTEROP_LOCAL_USERNAME)
            .cloned()
            .flatten(),
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn pump_and_apply(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) {
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    for action in handle.drain_actions() {
        state.apply(action);
    }
}

fn request_local_ready(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let action = if ready {
        GuiShellAction::AnnounceLocalUserReady
    } else {
        GuiShellAction::AnnounceLocalUserNotReady
    };
    if !state.apply(action) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to apply local readiness action; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetLocalReady(ready));
    Ok(())
}

fn request_remote_user_ready(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    username: &str,
    ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::RequestMainWindowUserReady {
        username: username.to_owned(),
        ready,
    }) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to stage remote readiness change for {username:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetReadyForUser {
        username: username.to_owned(),
        ready,
    });
    Ok(())
}

fn request_local_room_join(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    room: &str,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::JoinMainWindowRoom(room.to_owned())) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to apply local room join {room:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetRoom(room.to_owned()));
    Ok(())
}

fn request_local_playlist_queue(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    entry: &str,
    select_after_queue: bool,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::UpdateNewPlaylistEntryDraft(
        entry.to_owned(),
    )) || !state.apply(GuiShellAction::CommitNewPlaylistEntry)
    {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to queue a local shared playlist entry {entry:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::QueuePlaylistEntry {
        entry: entry.to_owned(),
        select_after_queue,
    });
    Ok(())
}

fn request_local_playlist_selection(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    index: usize,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::SelectMainWindowPlaylist(index)) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to select local shared playlist index {index}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(index));
    Ok(())
}

fn request_local_playlist_remove_selected(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let Some(index) = state.selection.selected_main_window_playlist else {
        return Err(LivePythonPeerInteropError::Gui(
            "failed to remove a local shared playlist entry because no row is selected.".to_owned(),
        ));
    };
    if !state.apply(GuiShellAction::RemoveSelectedMainWindowPlaylist) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to remove local shared playlist index {index}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::DeletePlaylistIndex(index));
    Ok(())
}

fn request_local_shared_playlist_open(handle: &GuiQueuedRuntimeBridgeHandle, paths: &[&str]) {
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
}

fn request_local_chat_send(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    message: &str,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::BeginLocalChatSend(message.to_owned())) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to stage local chat send {message:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage(message.to_owned()),
    ));
    Ok(())
}

fn wait_for_peer_observed_user_ready(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    ready: bool,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_ready(username, ready, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_user_controller(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    controller: bool,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_controller(username, controller, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_chat_message(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    message: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_chat_message(username, message, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_user_file_name(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    file_name: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_file_name(username, file_name, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_playlist(
    harness: &mut LegacyServerPythonPeerHarness,
    playlist: &[String],
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_playlist(playlist, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_playlist_index(
    harness: &mut LegacyServerPythonPeerHarness,
    index: usize,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_playlist_index(index, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_user_presence(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = harness.peer_snapshot()?;
        if snapshot.observed_users.contains_key(username) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for Python reference peer to observe GUI user {username:?}; users={:?}",
                snapshot.observed_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_sustained_connection_presence(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    duration: Duration,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + duration;
    loop {
        pump_and_apply(owner, handle, state);
        let peer_snapshot = harness.peer_snapshot()?;
        let local_present = local_user_ready(state).is_some();
        let peer_observes_local = peer_snapshot.observed_users.contains_key(username);
        if !local_present || !peer_observes_local {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "live Python detached-connect keepalive dropped before {duration:?}; room={:?}, projected_local_present={}, peer_users={:?}",
                state.main_window.room_name, local_present, peer_snapshot.observed_users
            )));
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projected_room_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    expected_room: &str,
    expected_peer_visible: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        let peer_visible = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == expected_room
            && local_user_ready(state).is_some()
            && peer_visible == expected_peer_visible
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python room projection; expected_room={expected_room:?}, expected_peer_visible={expected_peer_visible}, actual_room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projected_chat_message(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    sender: &str,
    message: &str,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if state
            .main_window
            .chat
            .iter()
            .any(|row| row.sender == sender && row.message == message)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let projected_chat = state
                .main_window
                .chat
                .iter()
                .map(|row| format!("{}>{}", row.sender, row.message))
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer chat projection {sender:?}>{message:?}; room={:?}, chat=[{}]",
                state.main_window.room_name, projected_chat
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projected_user_absence(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    username: &str,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if local_user_ready(state).is_some() && peer_user_ready(state, username).is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer disconnect projection; username={username:?}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_playlist_controls(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if state.main_window.playback.can_manage_playlist {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist controls; room={:?}, can_manage_playlist={}, playlist=[{}]",
                state.main_window.room_name,
                state.main_window.playback.can_manage_playlist,
                gui_playlist(state).join(" | ")
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projected_playlist(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    expected_playlist: &[String],
    expected_index: Option<usize>,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if gui_playlist(state) == expected_playlist
            && state.selection.selected_main_window_playlist == expected_index
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist projection; expected_playlist={expected_playlist:?}, expected_index={expected_index:?}, actual_playlist={:?}, actual_index={:?}",
                gui_playlist(state),
                state.selection.selected_main_window_playlist
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    expected_local_ready: bool,
    expected_peer_ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_ready_matches = local_user_ready(state) == Some(expected_local_ready);
        let peer_ready_matches =
            peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(expected_peer_ready);
        let local_user_present = local_user_ready(state).is_some();
        let peer_user_present = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_ROOM
            && local_user_present
            && peer_user_present
            && local_ready_matches
            && peer_ready_matches
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer readiness projection; expected_local_ready={expected_local_ready}, expected_peer_ready={expected_peer_ready}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_controlled_room_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_user_present = local_user_controller(state).is_some();
        let peer_user_present =
            peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
            && local_user_present
            && peer_user_present
            && local_user_controller(state) == Some(true)
            && peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(false)
            && state.main_window.playback.can_manage_playlist
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python controlled-room projection; room={:?}, can_manage_playlist={}, users=[{}]",
                state.main_window.room_name,
                state.main_window.playback.can_manage_playlist,
                projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_controlled_room_peer_ready_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    expected_peer_ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_user_present = local_user_controller(state).is_some();
        let peer_user_present = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
            && local_user_present
            && peer_user_present
            && local_user_controller(state) == Some(true)
            && peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(false)
            && peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME)
                == Some(expected_peer_ready)
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python controlled-room readiness projection; expected_peer_ready={expected_peer_ready}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn local_user_ready(state: &SyncplayGuiShellAppState) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self)
        .map(|user| user.is_ready)
}

fn local_user_controller(state: &SyncplayGuiShellAppState) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self)
        .map(|user| user.is_controller)
}

fn peer_user_ready(state: &SyncplayGuiShellAppState, username: &str) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| {
            user.username == username
                && !user.is_self
                && user.room_name == state.main_window.room_name
        })
        .map(|user| user.is_ready)
}

fn peer_user_controller(state: &SyncplayGuiShellAppState, username: &str) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| {
            user.username == username
                && !user.is_self
                && user.room_name == state.main_window.room_name
        })
        .map(|user| user.is_controller)
}

fn gui_playlist(state: &SyncplayGuiShellAppState) -> Vec<String> {
    state
        .main_window
        .playlist
        .iter()
        .map(|row| row.label.clone())
        .collect()
}

fn merge_peer_chat_messages(
    destination: &mut Vec<LegacyPythonPeerChatMessage>,
    additional: Vec<LegacyPythonPeerChatMessage>,
) {
    for message in additional {
        if !destination.iter().any(|existing| existing == &message) {
            destination.push(message);
        }
    }
}
