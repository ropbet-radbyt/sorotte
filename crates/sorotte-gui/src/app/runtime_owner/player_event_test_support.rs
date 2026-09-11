//! Physical observations for GUI runtime scenarios. Callers choose the media
//! episode and supply every observed value; absent clocks remain absent.
pub(super) use sorotte_player_api::scripted_events::ScriptedPlayerEvents;
use sorotte_player_api::*;

pub(super) fn active_player_events(generation: u64) -> ScriptedPlayerEvents {
    let mut events = ScriptedPlayerEvents::new(PlayerAttachmentEpoch::new(1));
    events.push_event(active_player_event(generation));
    events
}

pub(super) fn active_player_event(generation: u64) -> PlayerEvent {
    PlayerEvent::LoadAttemptActive {
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        command_id: None,
        playlist_entry_id: generation as i64,
    }
}

pub(super) fn file_event(generation: u64, update: LocalFileUpdate) -> PlayerEvent {
    PlayerEvent::LocalFileChanged {
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        update,
    }
}

pub(super) fn playback_event(
    generation: u64,
    update: PlayerPlaybackTelemetryUpdate,
) -> PlayerEvent {
    PlayerEvent::TransportDelta(PlayerTransportDelta {
        load_attempt_id: Some(LoadAttemptId::new(generation)),
        media_generation: Some(PlayerMediaGeneration::new(generation)),
        logical_pause: update.paused,
        position_seconds: update.position_seconds,
        playback_rate: update.playback_rate,
        paused_for_cache: update.paused_for_cache,
        cache_percentage: update.cache_buffering_percent,
        ..Default::default()
    })
}

pub(super) fn transport_event(update: PlayerTransportTelemetryUpdate) -> PlayerEvent {
    let mut delta = PlayerTransportDelta::from(update);
    delta.load_attempt_id = delta
        .media_generation
        .map(|generation| LoadAttemptId::new(generation.get()));
    PlayerEvent::TransportDelta(delta)
}

pub(super) fn starting_player_event(
    generation: u64,
    command_id: Option<PlayerCommandId>,
) -> PlayerEvent {
    PlayerEvent::LoadAttemptStarting {
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        command_id,
        playlist_entry_id: generation as i64,
        owns_transport: true,
    }
}

pub(super) fn load_succeeded(
    generation: u64,
    command_id: Option<PlayerCommandId>,
    requested_target: impl Into<String>,
    loaded_target: Option<String>,
) -> PlayerSemanticOutcome {
    PlayerSemanticOutcome::LoadAttempt(LoadAttemptOutcome {
        attachment_epoch: PlayerAttachmentEpoch::new(1),
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        command_id,
        requested_target: requested_target.into(),
        loaded_target,
        result: PlayerLoadAttemptResult::Loaded,
    })
}

pub(super) fn load_failed(
    generation: u64,
    command_id: Option<PlayerCommandId>,
    requested_target: impl Into<String>,
    loaded_target: Option<String>,
    kind: PlayerMediaLoadFailureKind,
) -> PlayerSemanticOutcome {
    PlayerSemanticOutcome::LoadAttempt(LoadAttemptOutcome {
        attachment_epoch: PlayerAttachmentEpoch::new(1),
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        command_id,
        requested_target: requested_target.into(),
        loaded_target,
        result: PlayerLoadAttemptResult::Failed(kind),
    })
}

pub(super) fn command_outcome(
    command_id: PlayerCommandId,
    media_generation: Option<PlayerMediaGeneration>,
    result: PlayerCommandSemanticResult,
) -> PlayerSemanticOutcome {
    PlayerSemanticOutcome::Command(PlayerCommandOutcome {
        attachment_epoch: PlayerAttachmentEpoch::new(1),
        command_id,
        media_generation,
        result,
    })
}

pub(super) fn bound_player_event(generation: u64, command_id: PlayerCommandId) -> PlayerEvent {
    PlayerEvent::LoadAttemptBound {
        attempt_id: LoadAttemptId::new(generation),
        media_generation: PlayerMediaGeneration::new(generation),
        command_id: Some(command_id),
        playlist_entry_id: generation as i64,
    }
}
