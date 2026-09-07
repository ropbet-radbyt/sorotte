//! A newer local Play/Pause can outlive the acknowledgement of its earlier
//! Seek. Correlation is captured from the emitted command and consumed once;
//! only an actually admitted canonical revision may rebase that newer intent.
use super::*;

impl RuntimePlaybackCoordination {
    /// A later Seek attempt cannot reuse an older command's acknowledgement,
    /// even when refreshing or dispatching the newer command fails.
    pub(crate) fn clear_local_seek_echo(&mut self) {
        self.pending_local_seek_echo = None;
        if let Some(intent) = self.pending_local_pause_intent.as_mut() {
            intent.preceding_local_seek = None;
        }
    }

    pub(super) fn local_seek_scope_matches(
        &self,
        seek: &PendingLocalSeekEcho,
        session: &ClientSession,
    ) -> bool {
        session.is_active()
            && session.room() == Some(seek.room.as_str())
            && session.username() == Some(seek.username.as_str())
            && self.connection_generation == seek.connection_generation
            && self.coordinator.current_media_generation() == Some(seek.local_media_generation)
            && self
                .participant_status
                .pending_participant_status_room_switch_target
                .is_none()
    }

    /// Arm only after both the player operation and its ordered State have
    /// been accepted. Capture the wire counter before any later heartbeat or
    /// server acknowledgement changes the session's ignore counters.
    pub(crate) fn record_emitted_local_seek(
        &mut self,
        session: &ClientSession,
        state: &StatePayload,
    ) {
        let Some(playstate) = state.playstate.as_ref() else {
            return;
        };
        let (Some(room), Some(username), Some(local_media_generation), Some(base_revision)) = (
            session.room(),
            session.username(),
            self.coordinator.current_media_generation(),
            session.current_room_transport_revision(),
        ) else {
            return;
        };
        let (Some(target_position), Some(paused), Some(client_counter)) = (
            playstate
                .position
                .filter(|position| position.is_finite() && *position >= 0.0),
            playstate.paused,
            state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.client),
        ) else {
            return;
        };
        if playstate.do_seek != Some(true)
            || playstate.transport_revision().ok().flatten() != Some(base_revision)
            || base_revision == 0
            || base_revision.checked_add(1).is_none()
            || client_counter == 0
            || client_counter == u32::MAX
        {
            return;
        }
        let seek = PendingLocalSeekEcho {
            room: room.to_owned(),
            username: username.to_owned(),
            local_media_generation,
            connection_generation: self.connection_generation,
            base_revision,
            target_position,
            paused,
            client_counter,
        };
        if self.local_seek_scope_matches(&seek, session) {
            // The legacy counter resets on server acknowledgement and
            // saturates. A repeated wire counter cannot distinguish two
            // same-revision Seeks, so it earns no special rebase authority.
            // Keep one bounded watermark across later Seek invalidation.
            if self
                .local_seek_counter_high_watermark
                .as_ref()
                .is_some_and(|previous| {
                    previous.room == seek.room
                        && previous.username == seek.username
                        && previous.local_media_generation == seek.local_media_generation
                        && previous.connection_generation == seek.connection_generation
                        && previous.base_revision == seek.base_revision
                        && client_counter <= previous.client_counter
                })
            {
                return;
            }
            self.local_seek_counter_high_watermark = Some(seek.clone());
            self.pending_local_seek_echo = Some(seek);
        }
    }

    /// Capture wire identity before Session consumes its ignore counters.
    /// Rejected traffic cannot consume the still-unacknowledged predecessor.
    pub(crate) fn capture_local_seek_echo(
        &mut self,
        session: &ClientSession,
        inbound: &ClientStateUpdate,
    ) -> Option<LocalSeekEchoCandidate> {
        let seek = self.pending_local_seek_echo.as_ref()?;
        let client_counter = inbound
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|ignore| ignore.client);
        let scope_matches = self.local_seek_scope_matches(seek, session)
            && session.current_room_transport_revision() == Some(seek.base_revision);
        if !scope_matches {
            self.clear_local_seek_echo();
            return None;
        }
        Some(LocalSeekEchoCandidate {
            seek: seek.clone(),
            matching_client_counter: client_counter == Some(seek.client_counter),
        })
    }

    /// Rebase after Session actually accepts this next revision. The first
    /// response still observes the normal post-revision player fence; the
    /// following heartbeat can publish the newer explicit Play/Pause.
    pub(crate) fn finish_local_seek_echo(
        &mut self,
        session: &ClientSession,
        candidate: Option<LocalSeekEchoCandidate>,
    ) {
        let Some(LocalSeekEchoCandidate {
            seek,
            matching_client_counter,
        }) = candidate
        else {
            return;
        };
        if self.pending_local_seek_echo.as_ref() != Some(&seek) {
            return;
        }
        let scope_matches = self.local_seek_scope_matches(&seek, session);
        if scope_matches && session.current_room_transport_revision() == Some(seek.base_revision) {
            // In particular, a matching client counter without an accepted
            // playstate is not an acknowledgement of the Seek's authority.
            return;
        }
        self.pending_local_seek_echo = None;
        // Both Session reconciliation paths require a complete position and
        // pause before admission and replace the canonical doSeek/setBy.
        // Validate that admitted transport once, using the counter captured
        // before Session reset it and the revision captured before admission.
        let admitted = matching_client_counter
            && scope_matches
            && session.current_room_transport_revision() == seek.base_revision.checked_add(1)
            && session.current_room_playstate().is_some_and(|playstate| {
                playstate.do_seek == Some(true)
                    && playstate.set_by.as_deref() == Some(seek.username.as_str())
                    && playstate.position == Some(seek.target_position)
                    && playstate.paused == Some(seek.paused)
            });
        let authorized = self
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| {
                intent.authorization == LocalIntentAuthorization::Authorized
                    && intent.connection_generation == seek.connection_generation
                    && intent.local_media_generation == seek.local_media_generation
                    && intent.room == seek.room
                    && intent.base_transport_revision == Some(seek.base_revision)
                    && session
                        .current_room_playstate_authority()
                        .is_some_and(|authority| {
                            self.room_authority_may_accept_local_pause_intent(
                                session,
                                authority,
                                intent.paused,
                            )
                        })
            });
        let Some(intent) = self.pending_local_pause_intent.as_mut() else {
            return;
        };
        let matching_predecessor = intent.preceding_local_seek.take().as_ref() == Some(&seek);
        if !matching_predecessor || !admitted || !authorized {
            return;
        }
        intent.base_transport_revision = seek.base_revision.checked_add(1);
        intent.last_canonical_playstate_updated_at_seconds = session
            .model
            .room
            .playstate_updated_at_seconds
            .get(&seek.room)
            .copied();
        intent.mismatching_canonical_playstate_updates = 0;
        intent.first_mismatching_canonical_playstate_at_seconds = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingLocalSeekEcho {
    room: String,
    username: String,
    local_media_generation: u64,
    connection_generation: u64,
    pub(super) base_revision: u64,
    target_position: f64,
    paused: bool,
    client_counter: u32,
}

#[derive(Debug)]
pub(crate) struct LocalSeekEchoCandidate {
    seek: PendingLocalSeekEcho,
    matching_client_counter: bool,
}
