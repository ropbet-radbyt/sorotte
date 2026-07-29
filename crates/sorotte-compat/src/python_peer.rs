use super::*;

const PYTHON_PEER_RESPONSE_DELIVERY_GRACE: Duration = Duration::from_secs(2);

fn python_peer_observation_response_timeout(observation_timeout: Duration) -> Duration {
    observation_timeout.saturating_add(PYTHON_PEER_RESPONSE_DELIVERY_GRACE)
}

mod api;
mod commands;
mod parsing;
mod peer_process;
mod server_process;
