use std::{
    thread,
    time::{Duration, Instant},
};

use sorotte_client_app::app_boundary::state::TlsPolicy;
use sorotte_compat::{LegacyPythonPeerChatMessage, LegacyServerPythonPeerHarness};

#[cfg(test)]
use super::super::{
    GuiOwnedPlayer, GuiPendingCompletionRequest, GuiRuntimeRequest, GuiTestPlayerAdapter,
};
use super::super::{
    GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle, GuiShellAction, GuiShellView,
    SorotteGuiShellAppState, StoredClientSettingsMvp,
};
use super::projection::{
    gui_playlist, local_user_controller, local_user_ready, merge_peer_chat_messages,
    peer_user_controller, peer_user_ready,
};
#[cfg(test)]
use super::runtime_actions::request_local_shared_playlist_open;
use super::runtime_actions::{
    pump_and_apply, request_local_chat_send, request_local_playlist_queue,
    request_local_playlist_remove_selected, request_local_playlist_selection, request_local_ready,
    request_local_room_join, request_remote_user_ready,
};
use super::waits::{
    wait_for_controlled_room_peer_ready_projection, wait_for_controlled_room_projection,
    wait_for_peer_observed_chat_message, wait_for_peer_observed_playlist,
    wait_for_peer_observed_playlist_index, wait_for_peer_observed_user_controller,
    wait_for_peer_observed_user_presence, wait_for_peer_observed_user_ready,
    wait_for_playlist_controls, wait_for_projected_chat_message, wait_for_projected_playlist,
    wait_for_projected_room_projection, wait_for_projected_user_absence, wait_for_projection,
};
#[cfg(test)]
use super::waits::{wait_for_peer_observed_user_file_name, wait_for_sustained_connection_presence};
use super::{
    LIVE_PYTHON_INTEROP_ALT_ROOM, LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT,
    LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE, LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE,
    LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO, LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE,
    LIVE_PYTHON_INTEROP_LOCAL_USERNAME, LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
    LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE, LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO,
    LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE, LIVE_PYTHON_INTEROP_PEER_USERNAME,
    LIVE_PYTHON_INTEROP_POLL_INTERVAL, LIVE_PYTHON_INTEROP_ROOM,
    LivePythonPeerControlledRoomInteropResult, LivePythonPeerInteropError,
    LivePythonPeerInteropResult,
};
#[cfg(test)]
use super::{
    LIVE_PYTHON_INTEROP_KEEPALIVE_OBSERVATION, LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE,
    LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO, LivePythonPeerDetachedConnectInteropResult,
    LivePythonPeerSharedPlaylistOpenInteropResult,
};

#[cfg(test)]
struct LivePythonSharedPlaylistMediaFixture {
    root: std::path::PathBuf,
    paths: [String; 2],
}

#[cfg(test)]
impl LivePythonSharedPlaylistMediaFixture {
    fn create() -> Result<Self, LivePythonPeerInteropError> {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| {
                LivePythonPeerInteropError::Gui(format!(
                    "could not derive a unique live-Python media fixture path: {error}"
                ))
            })?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-live-python-shared-playlist-{}-{unique_suffix}",
            std::process::id()
        ));
        let first = root.join(LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE);
        let second = root.join(LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO);
        let fixture = Self {
            root,
            paths: [
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
        };
        std::fs::create_dir_all(&fixture.root).map_err(|error| {
            LivePythonPeerInteropError::Gui(format!(
                "could not create the live-Python media fixture directory: {error}"
            ))
        })?;
        for path in &fixture.paths {
            std::fs::write(path, b"live Python shared-playlist fixture").map_err(|error| {
                LivePythonPeerInteropError::Gui(format!(
                    "could not create live-Python media fixture {path:?}: {error}"
                ))
            })?;
        }
        Ok(fixture)
    }

    fn path_refs(&self) -> [&str; 2] {
        [&self.paths[0], &self.paths[1]]
    }
}

#[cfg(test)]
impl Drop for LivePythonSharedPlaylistMediaFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) fn run_live_python_peer_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_ROOM,
            harness.address(),
            TlsPolicy::PreferTls,
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    state.apply(GuiShellAction::SwitchView(GuiShellView::Room));
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

pub(super) fn run_live_python_peer_controlled_room_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerControlledRoomInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT,
            harness.address(),
            // This legacy-Python loopback fixture intentionally exercises a
            // credential-bearing plaintext protocol peer.
            TlsPolicy::Plaintext,
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    state.apply(GuiShellAction::SwitchView(GuiShellView::Room));
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

#[cfg(test)]
pub(super) fn run_live_python_peer_detached_public_server_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        GuiPendingCompletionRequest::from_state(&state)
            .expect("staged public-server connect should capture its submitted request"),
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

    state.apply(GuiShellAction::SwitchView(GuiShellView::Room));
    Ok(LivePythonPeerDetachedConnectInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        widget_count: state.shell_widget_tree().node_count(),
    })
}

#[cfg(test)]
pub(super) fn run_live_python_peer_startup_saved_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerDetachedConnectInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    state.apply(GuiShellAction::SwitchView(GuiShellView::Room));
    Ok(LivePythonPeerDetachedConnectInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        widget_count: state.shell_widget_tree().node_count(),
    })
}

#[cfg(test)]
pub(super) fn run_live_python_peer_shared_playlist_open_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerSharedPlaylistOpenInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_ROOM,
            harness.address(),
            TlsPolicy::PreferTls,
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    let expected_playlist = vec![
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE.to_owned(),
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_TWO.to_owned(),
    ];
    let media_fixture = LivePythonSharedPlaylistMediaFixture::create()?;
    request_local_shared_playlist_open(&handle, &media_fixture.path_refs());
    wait_for_projected_playlist(&mut owner, &handle, &mut state, &expected_playlist, Some(0))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_playlist(harness, &expected_playlist, Duration::from_secs(3))?;
    wait_for_peer_observed_playlist_index(harness, 0, Duration::from_secs(3))?;

    request_local_ready(&handle, &mut state, true)?;
    wait_for_projection(&mut owner, &handle, &mut state, true, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        true,
        Duration::from_secs(3),
    )?;
    let peer_snapshot = wait_for_peer_observed_user_file_name(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_OPEN_MEDIA_FILE_ONE,
        Duration::from_secs(3),
    )?;

    state.apply(GuiShellAction::SwitchView(GuiShellView::Room));
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
