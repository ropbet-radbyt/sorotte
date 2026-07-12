use super::*;

use crate::outbox::EffectOutbox;

mod accessors;
mod lifecycle_actions;
mod local_actions;
mod playback_coordination;
mod queued_control;

pub use accessors::{ClientPlayerIo, ClientSessionUpdate};
use playback_coordination::RuntimePlaybackCoordination;
pub use playback_coordination::{
    PlaybackBarrierRoomBufferingConfig, PlaybackBarrierStartConfig, PlaybackBarrierTimeoutAction,
    PlaybackCoordinationSnapshot,
};

#[derive(Debug)]
pub struct ClientRuntime<P, C> {
    session: ClientSession,
    player: P,
    control: C,
    pub(crate) ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible,
    pending_player_playback_telemetry_updates: EffectOutbox<PlayerPlaybackTelemetryUpdate>,
    last_local_file_update: Option<LocalFileUpdate>,
    playback_coordination: RuntimePlaybackCoordination,
}
