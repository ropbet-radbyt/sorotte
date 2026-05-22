mod cli;

use std::env;
use std::sync::Arc;

use anyhow::{Context, bail};
use cli::{
    CliAction, ServerBindEndpoint, ServerBindFamily, ServerRunConfig, parse_server_cli_args,
    print_help, print_version, resolve_run_config,
};
use sorotte_server::{ServerApp, run_server_network_loops_until_shutdown};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, watch};

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
    app.runtime_mut()
        .set_stats_snapshot_start_delay_for_port(config.port);
    app.runtime_mut().set_tls_cert_path(config.tls_cert_path);
    app.runtime_mut().set_stats_db_path(config.stats_db_file)?;

    let runtime = Arc::new(Mutex::new(std::mem::take(app.runtime_mut())));
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    run_server_network_loops_until_shutdown(listeners, runtime, None, shutdown_rx).await?;
    Ok(())
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
