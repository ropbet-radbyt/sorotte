use crate::PRIVACY_HIDDEN_FILENAME;
use crate::SEEK_THRESHOLD_SECONDS;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::{
    AutoplayCountdownNotification, ChatConfig, ChatNotification, ClientPingMetricsLegacyCompatible,
    ClientRuntime, ClientRuntimeAction, ClientRuntimeControl, ClientSession,
    ControlledRoomCreationNotification, ControllerAuthTransitionNotification,
    DesyncCorrectionAction, FileDifferenceSummary, LEGACY_CHAT_MAX_MESSAGE_LENGTH,
    LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS, LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH,
    LEGACY_FALLBACK_MAX_FILENAME_LENGTH, LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH,
    LEGACY_FALLBACK_MAX_USERNAME_LENGTH, PrivacyMode, QueuedRuntimeControl,
    ReadinessAutoplayConfig, ReconnectStateRestoreCorrectionMetrics,
    ReconnectStateRestoreCorrectionPolicyMode, ReconnectTransitionNotification, RoomPlaystateView,
    UnpauseActionMode, UserChangeNotification, unix_wall_clock_time_seconds_legacy_compatible,
};
use syncplay_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
};
use syncplay_protocol::{
    ChatPayload, IgnoringOnTheFlyPayload, ListPayload, PingPayload, PlaystatePayload,
    ProtocolError, ProtocolMessage, StatePayload, decode_line, decode_message_line,
};

fn scenario_fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("scenarios");
    path.push(name);
    path
}

fn replay_python_trace_fixture(name: &str) -> BTreeMap<String, ClientSession> {
    let fixture = fs::read_to_string(scenario_fixture_path(name))
        .expect("python trace fixture should be readable");
    let trace = decode_line(&fixture).expect("python trace fixture should be valid JSON");
    let steps = trace
        .get("steps")
        .and_then(|value| value.as_array())
        .expect("python trace fixture should contain steps array");

    let mut sessions = BTreeMap::new();
    for step in steps {
        let outputs = step
            .get("outputs")
            .and_then(|value| value.as_array())
            .expect("python trace step should contain outputs array");
        for output in outputs {
            let client_id = output
                .get("client")
                .and_then(|value| value.as_str())
                .expect("python trace output should contain client id");
            let message_line = output
                .get("message")
                .expect("python trace output should contain message")
                .to_string();
            let message = decode_message_line(&message_line)
                .expect("python trace output message should decode");
            sessions
                .entry(client_id.to_owned())
                .or_insert_with(ClientSession::default)
                .apply_protocol_message(message)
                .expect("python trace output should apply cleanly");
        }
    }

    sessions
}

struct DesyncRuntimeScenarioStep {
    now_seconds: f64,
    local_position: f64,
    local_can_control: bool,
    dont_slow_down_with_me: bool,
    speed_supported: bool,
    expected_actions: Vec<ClientRuntimeAction>,
}

fn run_desync_runtime_scenario(session: &mut ClientSession, steps: &[DesyncRuntimeScenarioStep]) {
    for (index, step) in steps.iter().enumerate() {
        let actions = session.runtime_actions_for_desync_correction(
            step.now_seconds,
            step.local_position,
            step.local_can_control,
            step.dont_slow_down_with_me,
            step.speed_supported,
        );
        assert_eq!(
            actions,
            step.expected_actions,
            "desync runtime scenario step {} actions mismatch",
            index + 1
        );
    }
}

fn desync_session_with_remote_state(
    global_position: f64,
    paused: bool,
    do_seek: bool,
    set_by: &str,
) -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    let state_line = json!({
        "State": {
            "playstate": {
                "position": global_position,
                "paused": paused,
                "doSeek": do_seek,
                "setBy": set_by,
            }
        }
    })
    .to_string();
    session
        .apply_message_json(&state_line)
        .expect("remote state should apply");
    session
}

#[derive(Default)]
struct RecordingPlayer {
    paused: Option<bool>,
    position: Option<f64>,
    playback_rate: Option<f64>,
    fail_set_paused: bool,
    fail_set_position: bool,
    pending_local_file_update: Option<LocalFileUpdate>,
    pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
    pending_chat_requests: std::collections::VecDeque<String>,
}

impl PlayerAdapter for RecordingPlayer {
    fn name(&self) -> &'static str {
        "recording-player"
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        if self.fail_set_paused {
            return Err(PlayerError::Unsupported("set_paused_failed"));
        }
        self.paused = Some(paused);
        Ok(())
    }

    fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
        if self.fail_set_position {
            return Err(PlayerError::Unsupported("set_position_failed"));
        }
        self.position = Some(position_seconds);
        Ok(())
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.playback_rate = Some(rate);
        Ok(())
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.pending_local_file_update.take()
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        self.pending_playback_telemetry_update.take()
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        self.pending_chat_requests.pop_front()
    }
}

#[derive(Default)]
struct RecordingRuntimeControl {
    room_updates: Vec<String>,
    ready_updates: Vec<(bool, bool)>,
    file_updates: Vec<Value>,
    playlist_updates: Vec<Vec<String>>,
    playlist_index_updates: Vec<i64>,
    state_updates: Vec<StatePayload>,
    controller_auth_requests: Vec<(String, String)>,
    chat_messages: Vec<String>,
    chat_notifications: Vec<ChatNotification>,
    controlled_room_creation_notifications: Vec<ControlledRoomCreationNotification>,
    controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    user_change_notifications: Vec<UserChangeNotification>,
    reconnect_schedules: Vec<f64>,
    stop_reconnect_calls: usize,
    reconnect_notifications: Vec<ReconnectTransitionNotification>,
}

impl ClientRuntimeControl for RecordingRuntimeControl {
    fn set_room(&mut self, room: String) {
        self.room_updates.push(room);
    }

    fn set_ready(&mut self, ready: bool, manually_initiated: bool) {
        self.ready_updates.push((ready, manually_initiated));
    }

    fn set_file(&mut self, file_payload: Value) {
        self.file_updates.push(file_payload);
    }

    fn set_playlist(&mut self, files: Vec<String>) {
        self.playlist_updates.push(files);
    }

    fn set_playlist_index(&mut self, index: i64) {
        self.playlist_index_updates.push(index);
    }

    fn send_state(&mut self, state: StatePayload) {
        self.state_updates.push(state);
    }

    fn request_controller_auth(&mut self, room: String, password: String) {
        self.controller_auth_requests.push((room, password));
    }

    fn send_chat(&mut self, message: String) {
        self.chat_messages.push(message);
    }

    fn notify_chat(&mut self, notification: ChatNotification) {
        self.chat_notifications.push(notification);
    }

    fn notify_controlled_room_creation(
        &mut self,
        notification: ControlledRoomCreationNotification,
    ) {
        self.controlled_room_creation_notifications
            .push(notification);
    }

    fn notify_controller_auth_transition(
        &mut self,
        notification: ControllerAuthTransitionNotification,
    ) {
        self.controller_auth_notifications.push(notification);
    }

    fn notify_user_change(&mut self, notification: UserChangeNotification) {
        self.user_change_notifications.push(notification);
    }

    fn schedule_reconnect(&mut self, delay_seconds: f64) {
        self.reconnect_schedules.push(delay_seconds);
    }

    fn stop_reconnect(&mut self) {
        self.stop_reconnect_calls += 1;
    }

    fn notify_reconnect_transition(&mut self, notification: ReconnectTransitionNotification) {
        self.reconnect_notifications.push(notification);
    }
}

mod chat_tests;
mod control_tests;
mod controller_tests;
mod file_metadata_tests;
mod ping_tests;
mod playback_sync_tests;
mod playlist_tests;
mod protocol_tests;
mod readiness_autoplay_tests;
mod reconnect_tests;
mod runtime_tests;
mod session_tests;
mod trace_tests;
