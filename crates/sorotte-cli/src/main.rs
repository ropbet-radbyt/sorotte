use sorotte_lifecycle_evidence::{
    Disposition, ProcessInventorySpec, ProcessRole, TargetKind, TransitionObservation, Trigger,
    emit_global, flush_global, init_global_from_env,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_global_from_env(ProcessInventorySpec::new(
        ProcessRole::Client,
        [ProcessRole::Client, ProcessRole::Player],
    )?)?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Client,
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
            ProcessRole::Client,
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

    let result = sorotte_cli::run_sorotte_cli_from_env().await;
    let (observed_effect, disposition) = if result.is_ok() {
        ("shutdown-requested", Disposition::Accepted)
    } else {
        ("runtime-failed", Disposition::Failed)
    };
    emit_global(
        TransitionObservation::new(
            ProcessRole::Client,
            "application-process",
            "application",
            "APP-STOP-001",
        )
        .target(TargetKind::ProcessBoundary)
        .triggered_by(Trigger::Shutdown)
        .authority("process-owned", "draining")
        .effect("bounded-drain", observed_effect)
        .disposition(disposition),
    )?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Client,
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
    flush_global()?;
    result
}
