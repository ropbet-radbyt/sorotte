#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use sorotte_lifecycle_evidence::{
    Disposition, EvidenceError, ProcessInventorySpec, ProcessRole, TargetKind,
    TransitionObservation, Trigger, emit_global, flush_global, init_global_from_env,
};

fn main() {
    if let Err(error) = run_with_lifecycle_evidence() {
        eprintln!("sorotte-gui lifecycle evidence failed: {error}");
        std::process::exit(1);
    }
}

fn run_with_lifecycle_evidence() -> Result<(), EvidenceError> {
    init_global_from_env(ProcessInventorySpec::new(
        ProcessRole::Gui,
        [ProcessRole::Gui, ProcessRole::Client, ProcessRole::Player],
    )?)?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Gui,
            "application-process",
            "application",
            "APP-LAUNCH-001",
        )
        .target(TargetKind::ProcessBoundary)
        .triggered_by(Trigger::Startup)
        .authority("unowned", "initializing")
        .effect("process-starting", "process-starting")
        .disposition(Disposition::Accepted),
    )?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Gui,
            "application-process",
            "application",
            "APP-RUN-001",
        )
        .target(TargetKind::ProcessBoundary)
        .triggered_by(Trigger::Startup)
        .authority("initializing", "process-owned")
        .effect("process-running", "process-running")
        .disposition(Disposition::Applied),
    )?;
    sorotte_gui::run_sorotte_gui();
    emit_global(
        TransitionObservation::new(
            ProcessRole::Gui,
            "application-process",
            "application",
            "APP-STOP-001",
        )
        .target(TargetKind::ProcessBoundary)
        .triggered_by(Trigger::Shutdown)
        .authority("process-owned", "draining")
        .effect("bounded-drain", "shutdown-requested")
        .disposition(Disposition::Accepted),
    )?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Gui,
            "application-process",
            "application",
            "APP-TERM-001",
        )
        .target(TargetKind::ProcessBoundary)
        .triggered_by(Trigger::Shutdown)
        .authority("draining", "unowned")
        .effect("resources-released", "resources-released")
        .disposition(Disposition::Applied),
    )?;
    flush_global()
}
