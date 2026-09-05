mod cli;

use std::{env, future::Future, io};

use anyhow::{Context, bail};
use cli::{
    CliAction, ServerBindEndpoint, ServerBindFamily, ServerRunConfig, parse_server_cli_args,
    print_help, print_version, resolve_run_config,
};
use sorotte_lifecycle_evidence::{
    Disposition, ProcessInventorySpec, ProcessRole, TargetKind, TransitionObservation, Trigger,
    emit_global, flush_global, init_global_from_env,
};
use sorotte_server::{ServerActorHandle, ServerApp, run_server_network_loops_and_shutdown_actor};
use tokio::net::TcpListener;
use tokio::sync::watch;

#[cfg(any(windows, test))]
async fn first_shutdown_notification<C, B>(ctrl_c: C, ctrl_break: B)
where
    C: Future<Output = ()>,
    B: Future<Output = ()>,
{
    tokio::pin!(ctrl_c);
    tokio::pin!(ctrl_break);
    tokio::select! {
        _ = &mut ctrl_c => {}
        _ = &mut ctrl_break => {}
    }
}

#[cfg(windows)]
async fn platform_shutdown_signal() -> io::Result<()> {
    // CTRL_BREAK is the targetable graceful event for a dedicated Windows
    // process group. Treat it exactly like interactive CTRL_C so supervisors
    // can request one server drain without signaling their own console group.
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    first_shutdown_notification(
        async move {
            let _ = ctrl_c.recv().await;
        },
        async move {
            let _ = ctrl_break.recv().await;
        },
    )
    .await;
    Ok(())
}

#[cfg(not(windows))]
async fn platform_shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

fn endpoint_label(endpoint: &ServerBindEndpoint) -> &'static str {
    match endpoint.family {
        ServerBindFamily::Ipv4 => "IPv4",
        ServerBindFamily::Ipv6 => "IPv6",
    }
}

async fn bind_server_listeners(config: &ServerRunConfig) -> anyhow::Result<Vec<TcpListener>> {
    let mut listeners = Vec::new();
    let mut failures = Vec::new();
    for endpoint in &config.bind_endpoints {
        match TcpListener::bind((endpoint.host.as_str(), config.port)).await {
            Ok(listener) => {
                let local_addr = listener
                    .local_addr()
                    .with_context(|| "failed to inspect listener local address")?;
                eprintln!("sorotte-server listening on {local_addr}");
                listeners.push(listener);
            }
            Err(source) => {
                failures.push(format!(
                    "{} {}:{} ({source})",
                    endpoint_label(endpoint),
                    endpoint.host,
                    config.port
                ));
            }
        }
    }

    if listeners.is_empty() {
        bail!(
            "unable to listen using configured IPv4/IPv6 endpoints: {}",
            failures.join("; ")
        );
    }
    for failure in failures {
        eprintln!("sorotte-server: listener bind failed: {failure}");
    }

    Ok(listeners)
}

async fn run_server_until_shutdown_signal<F>(
    listeners: Vec<TcpListener>,
    runtime: ServerActorHandle,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: Future<Output = io::Result<()>>,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let lifecycle =
        run_server_network_loops_and_shutdown_actor(listeners, runtime, None, shutdown_rx);
    tokio::pin!(lifecycle);
    tokio::pin!(shutdown_signal);

    tokio::select! {
        lifecycle_result = &mut lifecycle => lifecycle_result.map_err(anyhow::Error::new),
        signal_result = &mut shutdown_signal => {
            if signal_result.is_ok() {
                eprintln!("sorotte-server: shutdown requested; draining client sessions");
            }
            let _ = shutdown_tx.send(true);
            let lifecycle_result = lifecycle.await;
            match (signal_result, lifecycle_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Ok(()), Err(lifecycle_error)) => Err(anyhow::Error::new(lifecycle_error)),
                (Err(signal_error), Ok(())) => {
                    Err(signal_error).context("failed to listen for the server shutdown signal")
                }
                (Err(signal_error), Err(lifecycle_error)) => bail!(
                    "failed to listen for the server shutdown signal: {signal_error}; server lifecycle also failed: {lifecycle_error}"
                ),
            }
        }
    }
}

async fn run_server(config: ServerRunConfig) -> anyhow::Result<()> {
    let listeners = bind_server_listeners(&config).await?;

    let mut app = match config.room_password_salt {
        Some(salt) => ServerApp::with_room_password_salt(salt),
        None => ServerApp::new(),
    };

    if let Some(template) = config.motd_template {
        app.runtime_mut().set_motd_template(Some(template));
    }
    app.runtime_mut()
        .set_server_password_token(config.server_password_token);
    app.runtime_mut()
        .set_persistent_rooms_db_path(config.rooms_db_file)?;
    app.runtime_mut()
        .set_permanent_rooms_file_path(config.permanent_rooms_file)?;
    app.runtime_mut()
        .set_persistent_rooms_enabled(config.persistent_rooms_enabled);
    app.runtime_mut().set_isolate_rooms(config.isolate_rooms);
    app.runtime_mut().set_chat_enabled(config.chat_enabled);
    app.runtime_mut()
        .set_readiness_enabled(config.readiness_enabled);
    if let Some(max_chat_message_length) = config.max_chat_message_length {
        app.runtime_mut()
            .set_max_chat_message_length(max_chat_message_length);
    }
    if let Some(max_username_length) = config.max_username_length {
        app.runtime_mut()
            .set_max_username_length(max_username_length);
    }
    if let Some(max_persistent_rooms) = config.max_persistent_rooms {
        app.runtime_mut()
            .set_max_persistent_rooms(max_persistent_rooms);
    }
    if let Some(max_persistent_rooms_per_identity) = config.max_persistent_rooms_per_identity {
        app.runtime_mut()
            .set_max_persistent_rooms_per_identity(max_persistent_rooms_per_identity);
    }
    if let Some(cooldown_seconds) = config.persistent_room_creation_cooldown_seconds {
        app.runtime_mut()
            .set_persistent_room_creation_cooldown_seconds(cooldown_seconds as f64);
    }
    if let Some(expiry_seconds) = config.persistent_room_inactivity_expiry_seconds {
        app.runtime_mut()
            .set_persistent_room_inactivity_expiry_seconds(expiry_seconds as f64);
    }
    app.runtime_mut()
        .set_stats_snapshot_start_delay_for_port(config.port);
    app.runtime_mut().set_tls_cert_path(config.tls_cert_path);
    app.runtime_mut().set_stats_db_path(config.stats_db_file)?;

    app.runtime_mut()
        .set_resource_limits(sorotte_server::ServerResourceLimits::from_environment()?)?;
    let runtime = ServerActorHandle::spawn(std::mem::take(app.runtime_mut()));
    emit_global(
        TransitionObservation::new(
            ProcessRole::Server,
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
    run_server_until_shutdown_signal(listeners, runtime, platform_shutdown_signal()).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_global_from_env(ProcessInventorySpec::new(
        ProcessRole::Server,
        [ProcessRole::Server],
    )?)?;
    emit_global(
        TransitionObservation::new(
            ProcessRole::Server,
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
    let result = match parse_server_cli_args(env::args().skip(1)) {
        Ok(CliAction::Help) => {
            print_help();
            Ok(())
        }
        Ok(CliAction::Version) => {
            print_version();
            Ok(())
        }
        Ok(CliAction::Run(overrides)) => run_server(resolve_run_config(*overrides)?).await,
        Err(error) => {
            eprintln!("sorotte-server: {error}");
            eprintln!("Try 'sorotte-server --help' for usage.");
            bail!("invalid command line")
        }
    };
    let (observed_effect, disposition) = if result.is_ok() {
        ("shutdown-requested", Disposition::Accepted)
    } else {
        ("runtime-failed", Disposition::Failed)
    };
    emit_global(
        TransitionObservation::new(
            ProcessRole::Server,
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
            ProcessRole::Server,
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

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use sorotte_server::{ServerActorError, ServerRuntime};

    use super::*;

    #[tokio::test]
    async fn platform_signal_selection_accepts_ctrl_c_and_ctrl_break_paths() {
        first_shutdown_notification(ready(()), pending::<()>()).await;
        first_shutdown_notification(pending::<()>(), ready(())).await;
    }

    #[tokio::test]
    async fn production_signal_boundary_runs_the_explicit_actor_shutdown_barrier() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let runtime = ServerActorHandle::spawn(ServerRuntime::new());
        let retained_probe = runtime.clone();

        run_server_until_shutdown_signal(vec![listener], runtime, ready(Ok(())))
            .await
            .expect("production lifecycle should shut down cleanly");

        assert!(
            matches!(
                retained_probe.session("probe").await,
                Err(ServerActorError::Unavailable)
            ),
            "retained actor sender proves production invoked shutdown instead of relying on Drop"
        );
    }

    #[tokio::test]
    async fn production_signal_boundary_surfaces_actor_shutdown_failure() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let runtime = ServerActorHandle::spawn(ServerRuntime::new());
        runtime
            .clone()
            .shutdown()
            .await
            .expect("precondition actor shutdown should succeed");

        let error = run_server_until_shutdown_signal(vec![listener], runtime, ready(Ok(())))
            .await
            .expect_err("production boundary must surface the failed durability barrier");

        assert!(
            error.to_string().contains("server actor shutdown"),
            "shutdown failure should remain explicit in the production error: {error:#}"
        );
    }
}
