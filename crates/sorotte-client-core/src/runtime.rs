use super::*;

use crate::outbox::EffectOutbox;

mod accessors;
mod lifecycle_actions;
mod local_actions;
mod playback_coordination;
mod queued_control;

pub use accessors::{ClientPlayerIo, ClientSessionUpdate};
pub use playback_coordination::{
    ExternalPlayerAvailability, PlaybackBarrierRoomBufferingConfig, PlaybackBarrierStartConfig,
    PlaybackBarrierTimeoutAction, PlaybackCoordinationSnapshot,
};
use playback_coordination::{OrderedPlayerEventConsumer, RuntimePlaybackCoordination};

#[derive(Debug)]
pub struct ClientRuntime<P, C> {
    session: ClientSession,
    player: P,
    control: C,
    pub(crate) ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible,
    pending_player_playback_telemetry_updates: EffectOutbox<PlayerPlaybackTelemetryUpdate>,
    pending_ordered_local_file_updates: EffectOutbox<LocalFileUpdate>,
    last_local_file_update: Option<LocalFileUpdate>,
    pending_reconnect_rate_reset: bool,
    playback_coordination: RuntimePlaybackCoordination,
    ordered_player_events: OrderedPlayerEventConsumer,
}
