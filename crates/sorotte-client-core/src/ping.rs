use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct ClientPingMetricsLegacyCompatible {
    client_rtt_seconds: f64,
    average_rtt_seconds: f64,
    server_rtt_seconds: f64,
    pub(crate) forward_delay_seconds: f64,
}

impl ClientPingMetricsLegacyCompatible {
    pub fn observe_inbound_state(&mut self, state: &StatePayload) {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.observe_inbound_state_at(state, now_seconds);
    }

    pub(crate) fn observe_inbound_state_at(&mut self, state: &StatePayload, now_seconds: f64) {
        let state = normalize_client_state_payload(state.clone());
        self.observe_normalized_inbound_state_at(&state, now_seconds);
    }

    pub(crate) fn observe_normalized_inbound_state(&mut self, state: &ClientStateUpdate) {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.observe_normalized_inbound_state_at(state, now_seconds);
    }

    pub(crate) fn observe_normalized_inbound_state_at(
        &mut self,
        state: &ClientStateUpdate,
        now_seconds: f64,
    ) {
        let Some(ping) = state.ping.as_ref() else {
            return;
        };
        let Some(client_latency_calculation) = ping.client_latency_calculation else {
            return;
        };
        let Some(server_rtt) = ping.server_rtt else {
            return;
        };
        if !client_latency_calculation.is_finite() || !server_rtt.is_finite() || server_rtt < 0.0 {
            return;
        }

        let current_rtt = now_seconds - client_latency_calculation;
        if !current_rtt.is_finite() || current_rtt < 0.0 {
            return;
        }
        self.client_rtt_seconds = current_rtt;
        self.server_rtt_seconds = server_rtt;
        if self.average_rtt_seconds == 0.0 {
            self.average_rtt_seconds = current_rtt;
        }
        self.average_rtt_seconds = self.average_rtt_seconds * LEGACY_PING_MOVING_AVERAGE_WEIGHT
            + current_rtt * (1.0 - LEGACY_PING_MOVING_AVERAGE_WEIGHT);
        self.forward_delay_seconds = if server_rtt < current_rtt {
            self.average_rtt_seconds / 2.0 + (current_rtt - server_rtt)
        } else {
            self.average_rtt_seconds / 2.0
        };
    }

    pub fn client_rtt_seconds(self) -> f64 {
        self.client_rtt_seconds
    }

    pub fn server_rtt_seconds(self) -> f64 {
        self.server_rtt_seconds
    }

    pub fn forward_delay_seconds(self) -> f64 {
        self.forward_delay_seconds
    }

    pub fn client_latency_calculation_now(self) -> f64 {
        let _ = self;
        unix_wall_clock_time_seconds_legacy_compatible()
    }
}

pub(crate) fn unix_wall_clock_time_seconds_legacy_compatible() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
