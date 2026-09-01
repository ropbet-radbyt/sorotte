use std::{fs::OpenOptions, io::Write, path::PathBuf};

use serde_json::{Value, json};

const TEST_LIFECYCLE_OBSERVATION_PATH_ENV: &str = "SOROTTE_GUI_TEST_LIFECYCLE_OBSERVATION_PATH";

pub(super) const EXIT_ACTION_APPLIED: &str = "exit-action-applied";
pub(super) const VIEWPORT_CLOSE_REQUESTED: &str = "viewport-close-requested";
pub(super) const RUNTIME_STOP_REQUESTED: &str = "runtime-stop-requested";
pub(super) const RUNTIME_WORKER_STOPPED: &str = "runtime-worker-stopped";
pub(super) const RUNTIME_SHUTDOWN_DEADLINE_EXCEEDED: &str = "runtime-shutdown-deadline-exceeded";
pub(super) const APP_DROP_COMPLETE: &str = "app-drop-complete";

fn observation_path() -> Option<PathBuf> {
    std::env::var_os(TEST_LIFECYCLE_OBSERVATION_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn record_value(value: Value) {
    let Some(path) = observation_path() else {
        return;
    };
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, &value)?;
        writeln!(file)?;
        file.flush()
    })();
    if let Err(error) = result {
        eprintln!(
            "sorotte-gui failed to record a test lifecycle event at {}: {error}",
            path.display()
        );
    }
}

pub(super) fn record(event: &str) {
    record_value(json!({
        "event": event,
        "pid": std::process::id(),
    }));
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PlaybackControlObservation {
    pub target_paused: Option<bool>,
    pub current_room_paused: Option<bool>,
    pub media_generation: Option<u64>,
    pub pending_local_pause_intent: Option<bool>,
    pub pending_local_pause_intent_dormant: bool,
    pub last_local_pause_intent_stage_accepted: Option<bool>,
    pub transport_telemetry_observed: bool,
    pub ordinary_correction_blocked: bool,
    pub playlist_reset_pending: bool,
    pub state_queued: Option<bool>,
}

/// Records a privacy-safe causal checkpoint for opt-in native verification.
/// Deliberately excludes room/user/media identity, URLs, and playback position.
pub(super) fn record_playback_control(event: &str, observation: PlaybackControlObservation) {
    record_value(json!({
        "event": event,
        "pid": std::process::id(),
        "target_paused": observation.target_paused,
        "current_room_paused": observation.current_room_paused,
        "media_generation": observation.media_generation,
        "pending_local_pause_intent": observation.pending_local_pause_intent,
        "pending_local_pause_intent_dormant": observation.pending_local_pause_intent_dormant,
        "last_local_pause_intent_stage_accepted": observation.last_local_pause_intent_stage_accepted,
        "transport_telemetry_observed": observation.transport_telemetry_observed,
        "ordinary_correction_blocked": observation.ordinary_correction_blocked,
        "playlist_reset_pending": observation.playlist_reset_pending,
        "state_queued": observation.state_queued,
    }));
}

pub(super) fn record_attached_pause_command(
    event: &str,
    target_paused: bool,
    source: &str,
    cause: &str,
    current_room_paused: Option<bool>,
    pending_local_pause_intent: Option<bool>,
    playlist_reset_pending: bool,
) {
    record_value(json!({
        "event": event,
        "pid": std::process::id(),
        "target_paused": target_paused,
        "source": source,
        "cause": cause,
        "current_room_paused": current_room_paused,
        "pending_local_pause_intent": pending_local_pause_intent,
        "playlist_reset_pending": playlist_reset_pending,
    }));
}
