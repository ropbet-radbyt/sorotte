use std::{fs::OpenOptions, io::Write, path::PathBuf};

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

pub(super) fn record(event: &str) {
    let Some(path) = observation_path() else {
        return;
    };
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(
            file,
            "{{\"event\":\"{event}\",\"pid\":{}}}",
            std::process::id()
        )?;
        file.flush()
    })();
    if let Err(error) = result {
        eprintln!(
            "sorotte-gui failed to record test lifecycle event {event:?} at {}: {error}",
            path.display()
        );
    }
}
