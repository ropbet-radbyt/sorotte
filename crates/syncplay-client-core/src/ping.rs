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
        let Some(ping) = state.ping.as_ref() else {
            return;
        };
        let Some(latency_calculation) = ping.latency_calculation else {
            return;
        };
        let sender_rtt = ping.client_rtt.unwrap_or(0.0);
        if !sender_rtt.is_finite() || sender_rtt < 0.0 {
            return;
        }
        let server_rtt = ping.server_rtt;
        if let Some(server_rtt_value) = server_rtt
            && (!server_rtt_value.is_finite() || server_rtt_value < 0.0)
        {
            return;
        }

        let current_rtt = now_seconds - latency_calculation;
        if !current_rtt.is_finite() || current_rtt < 0.0 {
            return;
        }
        self.client_rtt_seconds = current_rtt;
        if let Some(server_rtt_value) = server_rtt {
            self.server_rtt_seconds = server_rtt_value;
        }
        if self.average_rtt_seconds == 0.0 {
            self.average_rtt_seconds = current_rtt;
        }
        self.average_rtt_seconds = self.average_rtt_seconds * LEGACY_PING_MOVING_AVERAGE_WEIGHT
            + current_rtt * (1.0 - LEGACY_PING_MOVING_AVERAGE_WEIGHT);
        self.forward_delay_seconds = if let Some(server_rtt_value) = server_rtt {
            if server_rtt_value < current_rtt {
                self.average_rtt_seconds / 2.0 + (current_rtt - server_rtt_value)
            } else {
                self.average_rtt_seconds / 2.0
            }
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
