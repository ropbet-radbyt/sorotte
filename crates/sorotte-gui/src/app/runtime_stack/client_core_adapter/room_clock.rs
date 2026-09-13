use super::*;

pub(super) struct GuiRoomClockSample {
    room: String,
    playstate: RoomPlaystateView,
    updated_at_seconds: Option<f64>,
    position_seconds: Option<f64>,
    sampled_at: Option<Instant>,
}

impl GuiClientCoreChatSessionRuntimeAdapter {
    pub(in crate::app::runtime_stack) fn room_clock_position(
        &mut self,
    ) -> (Option<f64>, Option<Instant>) {
        let session = self.runtime.session();
        let (Some(room), Some(playstate)) = (session.room(), session.current_room_playstate())
        else {
            self.room_clock = None;
            return (None, None);
        };
        let updated_at_seconds = session.current_room_playstate_updated_at_seconds();
        if let Some(sample) = &self.room_clock
            && sample.room == room
            && sample.playstate == *playstate
            && sample.updated_at_seconds == updated_at_seconds
        {
            return (sample.position_seconds, sample.sampled_at);
        }

        // Anchor display interpolation once per source sample. Reanchoring every
        // poll makes an unchanged session look like fresh GUI input indefinitely.
        let position_seconds = session
            .current_room_playstate_at(system_time_seconds())
            .and_then(|playstate| playstate.position);
        let sampled_at =
            (playstate.paused == Some(false) && position_seconds.is_some()).then(Instant::now);
        self.room_clock = Some(GuiRoomClockSample {
            room: room.to_owned(),
            playstate: playstate.clone(),
            updated_at_seconds,
            position_seconds,
            sampled_at,
        });
        (position_seconds, sampled_at)
    }
}
