use std::env;
use syncplay_server::ServerApp;

fn env_flag_enabled(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

fn env_trimmed(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn env_u16(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn main() -> anyhow::Result<()> {
    let motd_template = env_trimmed("SYNCPLAY_SERVER_MOTD_TEMPLATE");
    let rooms_db_file = env_trimmed("SYNCPLAY_SERVER_ROOMS_DB_FILE");
    let permanent_rooms_file = env_trimmed("SYNCPLAY_SERVER_PERMANENT_ROOMS_FILE");
    let tls_cert_path = env_trimmed("SYNCPLAY_SERVER_TLS_CERT_PATH");
    let stats_db_file = env_trimmed("SYNCPLAY_SERVER_STATS_DB_FILE");
    let server_port = env_u16("SYNCPLAY_SERVER_PORT");
    let persistent_rooms_enabled = env_flag_enabled("SYNCPLAY_SERVER_PERSISTENT_ROOMS");
    let mut app = ServerApp::new();
    if let Some(template) = motd_template {
        app.runtime_mut().set_motd_template(Some(template));
    }
    app.runtime_mut()
        .set_persistent_rooms_db_path(rooms_db_file.as_deref().map(std::path::PathBuf::from))?;
    app.runtime_mut().set_permanent_rooms_file_path(
        permanent_rooms_file
            .as_deref()
            .map(std::path::PathBuf::from),
    )?;
    app.runtime_mut()
        .set_persistent_rooms_enabled(persistent_rooms_enabled || rooms_db_file.is_some());
    if let Some(port) = server_port {
        app.runtime_mut()
            .set_stats_snapshot_start_delay_for_port(port);
    }
    app.runtime_mut()
        .set_tls_cert_path(tls_cert_path.as_deref().map(std::path::PathBuf::from));
    app.runtime_mut()
        .set_stats_db_path(stats_db_file.as_deref().map(std::path::PathBuf::from))?;
    app.bootstrap_room("default");
    tracing::info!("syncplay-server phase0 bootstrap complete");
    println!(
        "syncplay-server bootstrap room: {}",
        app.room_is_present("default")
    );
    Ok(())
}
