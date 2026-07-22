mod cli;

use std::{env, future::Future, io};

use anyhow::{Context, bail};
use cli::{
    CliAction, ServerBindEndpoint, ServerBindFamily, ServerRunConfig, parse_server_cli_args,
    print_help, print_version, resolve_run_config,
};
use sorotte_server::{ServerActorHandle, ServerApp, run_server_network_loops_and_shutdown_actor};
use tokio::net::TcpListener;
use tokio::sync::watch;

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

    let runtime = ServerActorHandle::spawn(std::mem::take(app.runtime_mut()));
    run_server_until_shutdown_signal(listeners, runtime, tokio::signal::ctrl_c()).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match parse_server_cli_args(env::args().skip(1)) {
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
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use sorotte_server::{ServerActorError, ServerRuntime};

    use super::*;

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
