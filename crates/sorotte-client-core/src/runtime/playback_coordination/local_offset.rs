//! Local player offsets change coordinates, never canonical room transport.
use super::*;
use std::borrow::Cow;

fn shifted_position(position: f64, delta: f64) -> f64 {
    if position.is_finite() {
        (position + delta).max(0.0)
    } else {
        position
    }
}

fn shift_transport_snapshot(transport: &mut PlayerTransportSnapshot, delta: f64) {
    if let SnapshotField::Known(position) = &mut transport.position_seconds {
        *position = shifted_position(*position, delta);
    }
    if let SnapshotField::Known(ranges) = &mut transport.seekable_ranges {
        for range in ranges {
            *range = range.shifted(delta);
        }
    }
    if let SnapshotField::Known(range) = &mut transport.known_live_seekable_window {
        *range = range.shifted(delta);
    }
}

impl RuntimePlaybackCoordination {
    fn shift_local_player_timeline(&mut self, delta: f64) {
        // Re-express retained evidence without manufacturing observation clocks.
        // The next real sample will describe the newly requested physical seek.
        if let Some(observation) = self.latest_observation.as_mut() {
            observation.position_seconds = observation
                .position_seconds
                .map(|position| shifted_position(position, delta));
            if let Some(ranges) = observation.seekable_ranges.as_mut() {
                for range in ranges {
                    *range = range.shifted(delta);
                }
            }
            observation.known_live_seekable_window = observation
                .known_live_seekable_window
                .map(|range| range.shifted(delta));
        }
        if let Some(position) = self.latest_position_observation.as_mut() {
            position.position_seconds = shifted_position(position.position_seconds, delta);
        }
        self.coordinator.shift_local_player_timeline(delta);
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn local_playback_offset_seconds(&self) -> f64 {
        self.local_playback_offset_seconds
    }

    /// The room coordinate used to change a local offset. An emitted seek is
    /// newer than its still-pending canonical echo, but cannot outrank a new
    /// canonical transport revision or a different playback membership.
    pub fn local_offset_room_position_at(&self, now_seconds: f64) -> f64 {
        self.playback_coordination
            .unacknowledged_local_seek
            .as_ref()
            .filter(|seek| {
                self.playback_coordination
                    .local_transport_scope_matches(seek, &self.session)
                    && self.session.current_room_transport_revision() == Some(seek.base_revision)
            })
            .map(|seek| seek.target_position)
            .or_else(|| {
                self.session
                    .current_room_playstate_at(now_seconds)
                    .and_then(|state| state.position)
            })
            .or(self.session.model.playback.local_position)
            .unwrap_or(0.0)
    }

    /// Convert a canonical target only at the actual player command boundary.
    pub fn player_position_for_room(&self, position_seconds: f64) -> f64 {
        shifted_position(position_seconds, self.local_playback_offset_seconds)
    }

    /// Apply a local synchronization offset without issuing a room seek.
    /// The setting survives transport reconnection and changes only on success.
    pub fn set_local_playback_offset_seconds(
        &mut self,
        offset_seconds: f64,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        if !offset_seconds.is_finite() || !now_seconds.is_finite() {
            return Err(PlayerError::OperationFailed(
                "local playback offset must be finite".to_owned(),
            ));
        }
        // Consume preceding physical samples in their original coordinates.
        self.drain_ordered_player_events(now_seconds)?;
        let room_position = self.local_offset_room_position_at(now_seconds);
        let player_target = room_position + offset_seconds;
        if !player_target.is_finite() {
            return Err(PlayerError::OperationFailed(
                "local playback offset exceeds the player timeline".to_owned(),
            ));
        }
        self.interrupt_playback_recovery(now_seconds)?;
        self.player
            .execute(PlayerCommand::SetPosition(player_target.max(0.0)))?;
        self.invalidate_pending_natural_playback_completion();
        let delta = self.local_playback_offset_seconds - offset_seconds;
        self.local_playback_offset_seconds = offset_seconds;
        // Sparse later deltas merge into this retained transport snapshot.
        // Keep its coordinates aligned without changing its observation clocks.
        shift_transport_snapshot(&mut self.ordered_player_events.transport, delta);
        self.playback_coordination
            .shift_local_player_timeline(delta);
        self.session.model.playback.local_position = self
            .session
            .model
            .playback
            .local_position
            .map(|position| shifted_position(position, delta));
        if let Some(update) = self.pending_player_playback_telemetry_updates.back_mut() {
            update.position_seconds = update
                .position_seconds
                .map(|position| shifted_position(position, delta));
        }
        Ok(true)
    }

    pub(in crate::runtime) fn project_player_batch_to_room<'a>(
        &self,
        batch: &'a PlayerEventBatch,
    ) -> Cow<'a, PlayerEventBatch> {
        if self.local_playback_offset_seconds == 0.0 {
            return Cow::Borrowed(batch);
        }
        let delta = -self.local_playback_offset_seconds;
        let mut projected = batch.clone();
        if let Some(snapshot) = projected.authoritative_snapshot.as_mut() {
            shift_transport_snapshot(&mut snapshot.transport, delta);
        }
        for event in &mut projected.events {
            if let PlayerEvent::TransportDelta(transport) = &mut event.event {
                transport.position_seconds = transport
                    .position_seconds
                    .map(|position| shifted_position(position, delta));
                if let Some(ranges) = transport.seekable_ranges.as_mut() {
                    for range in ranges {
                        *range = range.shifted(delta);
                    }
                }
                transport.known_live_seekable_window = transport
                    .known_live_seekable_window
                    .map(|range| range.shifted(delta));
            }
        }
        Cow::Owned(projected)
    }
}
