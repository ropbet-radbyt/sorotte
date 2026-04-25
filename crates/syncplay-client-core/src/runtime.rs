use super::*;

mod accessors;
mod lifecycle_actions;
mod local_actions;
mod queued_control;

#[derive(Debug)]
pub struct ClientRuntime<P, C> {
    session: ClientSession,
    player: P,
    control: C,
    pub(crate) ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible,
    pending_player_playback_telemetry_updates: Vec<PlayerPlaybackTelemetryUpdate>,
}
