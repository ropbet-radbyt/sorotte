use super::*;
use crate::redacted_debug::RedactedJsonMap;

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playstate: Option<PlaystatePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping: Option<PingPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "ignoringOnTheFly"
    )]
    pub ignoring_on_the_fly: Option<IgnoringOnTheFlyPayload>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for StatePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StatePayload")
            .field("playstate", &self.playstate)
            .field("ping", &self.ping)
            .field("ignoring_on_the_fly", &self.ignoring_on_the_fly)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl StatePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_playstate(mut self, playstate: PlaystatePayload) -> Self {
        self.playstate = Some(playstate);
        self
    }

    pub fn with_ping(mut self, ping: PingPayload) -> Self {
        self.ping = Some(ping);
        self
    }

    pub fn with_ignoring_on_the_fly(
        mut self,
        ignoring_on_the_fly: IgnoringOnTheFlyPayload,
    ) -> Self {
        self.ignoring_on_the_fly = Some(ignoring_on_the_fly);
        self
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaystatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "doSeek")]
    pub do_seek: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "setBy")]
    pub set_by: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for PlaystatePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaystatePayload")
            .field("position", &self.position)
            .field("paused", &self.paused)
            .field("do_seek", &self.do_seek)
            .field("set_by", &self.set_by)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl PlaystatePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_position(mut self, position: f64) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_paused(mut self, paused: bool) -> Self {
        self.paused = Some(paused);
        self
    }

    pub fn with_do_seek(mut self, do_seek: bool) -> Self {
        self.do_seek = Some(do_seek);
        self
    }

    pub fn with_set_by(mut self, set_by: impl Into<String>) -> Self {
        self.set_by = Some(set_by.into());
        self
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PingPayload {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "latencyCalculation"
    )]
    pub latency_calculation: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "clientLatencyCalculation"
    )]
    pub client_latency_calculation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "clientRtt")]
    pub client_rtt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "serverRtt")]
    pub server_rtt: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for PingPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PingPayload")
            .field("latency_calculation", &self.latency_calculation)
            .field(
                "client_latency_calculation",
                &self.client_latency_calculation,
            )
            .field("client_rtt", &self.client_rtt)
            .field("server_rtt", &self.server_rtt)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl PingPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_latency_calculation(mut self, latency_calculation: f64) -> Self {
        self.latency_calculation = Some(latency_calculation);
        self
    }

    pub fn with_client_latency_calculation(mut self, client_latency_calculation: f64) -> Self {
        self.client_latency_calculation = Some(client_latency_calculation);
        self
    }

    pub fn with_client_rtt(mut self, client_rtt: f64) -> Self {
        self.client_rtt = Some(client_rtt);
        self
    }

    pub fn with_server_rtt(mut self, server_rtt: f64) -> Self {
        self.server_rtt = Some(server_rtt);
        self
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IgnoringOnTheFlyPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<u32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for IgnoringOnTheFlyPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IgnoringOnTheFlyPayload")
            .field("server", &self.server)
            .field("client", &self.client)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl IgnoringOnTheFlyPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_server(mut self, server: u32) -> Self {
        self.server = Some(server);
        self
    }

    pub fn with_client(mut self, client: u32) -> Self {
        self.client = Some(client);
        self
    }
}
