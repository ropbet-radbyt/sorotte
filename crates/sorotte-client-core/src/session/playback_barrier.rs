use serde_json::{Map, Value};
use sorotte_protocol::{
    CommitStartPayload, MediaLoadIntent, MediaReadyPayload, PlaybackBarrierPhase,
    PlaybackBarrierSetExtension, PlaybackBarrierStateExtension, PlaybackBarrierStatusPayload,
    PrepareMediaPayload, RoomBufferingPhase, RoomBufferingPolicy, RoomBufferingPolicyPayload,
    RoomBufferingStatusPayload, SOROTTE_PLAYBACK_BARRIER_V1, StartedAckPayload, StatePayload,
    TransportBufferingReportPayload,
};

use super::{ClientSession, ConnectionPhase};

#[derive(Debug, Default)]
pub(super) struct ClientPlaybackBarrierState {
    prepare: Option<PrepareMediaPayload>,
    commit: Option<CommitStartPayload>,
    status: Option<PlaybackBarrierStatusPayload>,
    buffering_policy: Option<RoomBufferingPolicyPayload>,
    buffering_status: Option<RoomBufferingStatusPayload>,
    last_transport_observation: Option<TransportBufferingReportPayload>,
    buffering_report_epoch: u64,
}

impl ClientSession {
    /// Adds the capability flag expected by a Sorotte playback-barrier server
    /// to an existing client Hello feature map.
    pub fn advertise_playback_barrier_v1(features: &mut Map<String, Value>) {
        features.insert(SOROTTE_PLAYBACK_BARRIER_V1.to_owned(), Value::Bool(true));
    }

    pub fn playback_barrier_v1_negotiated(&self) -> bool {
        matches!(
            &self.model.connection.phase,
            ConnectionPhase::Active(capabilities) if capabilities.playback_barrier_v1
        )
    }

    pub fn playback_barrier_prepare(&self) -> Option<&PrepareMediaPayload> {
        self.playback_barrier_v1_negotiated()
            .then_some(self.playback_barrier.prepare.as_ref())
            .flatten()
    }

    pub fn playback_barrier_commit(&self) -> Option<&CommitStartPayload> {
        self.playback_barrier_v1_negotiated()
            .then_some(self.playback_barrier.commit.as_ref())
            .flatten()
    }

    /// Returns a commit only while the server status says that commit is the
    /// active authority for desired playback. Retained commits remain
    /// available through `playback_barrier_commit` for diagnostics.
    pub fn playback_barrier_active_commit(&self) -> Option<&CommitStartPayload> {
        let commit = self.playback_barrier_commit()?;
        let status = self.playback_barrier_status()?;
        (status.phase == PlaybackBarrierPhase::Committed
            && status.media_generation == commit.media_generation
            && status.state_revision == Some(commit.state_revision))
        .then_some(commit)
    }

    pub fn playback_barrier_status(&self) -> Option<&PlaybackBarrierStatusPayload> {
        self.playback_barrier_v1_negotiated()
            .then_some(self.playback_barrier.status.as_ref())
            .flatten()
    }

    /// Returns the server-accepted, generation-scoped ongoing buffering
    /// policy. This state is independent of start-barrier prepare/commit
    /// transitions and remains available for the full media generation.
    pub fn playback_barrier_buffering_policy(&self) -> Option<&RoomBufferingPolicyPayload> {
        self.playback_barrier_v1_negotiated()
            .then_some(self.playback_barrier.buffering_policy.as_ref())
            .flatten()
    }

    /// Returns the latest validated server projection of the ongoing room
    /// buffering policy and eligible cohort.
    pub fn playback_barrier_buffering_status(&self) -> Option<&RoomBufferingStatusPayload> {
        self.playback_barrier_v1_negotiated()
            .then_some(self.playback_barrier.buffering_status.as_ref())
            .flatten()
    }

    /// Changes whenever the server sends a valid full buffering-policy
    /// snapshot. Runtime-level reporting uses this epoch so an identical
    /// policy received by a new room/connection transport still produces one
    /// fresh current-state report.
    pub(crate) fn playback_barrier_buffering_report_epoch(&self) -> u64 {
        self.playback_barrier.buffering_report_epoch
    }

    /// Builds a transport-readiness observation only while the reported media
    /// generation is the currently prepared, not-yet-committed generation.
    /// This is deliberately unrelated to Syncplay's user `ready` state.
    pub fn playback_barrier_media_ready_observation(
        &self,
        media_generation: u64,
        loaded: bool,
        seekable: Option<bool>,
        buffer_ready: bool,
    ) -> Option<StatePayload> {
        let prepare = self.playback_barrier_prepare()?;
        if media_generation == 0
            || prepare.media_generation != media_generation
            || self
                .playback_barrier_status()
                .is_none_or(|status| status.phase != PlaybackBarrierPhase::Preparing)
        {
            return None;
        }

        let mut ready = MediaReadyPayload::new(media_generation, loaded, buffer_ready);
        if let Some(seekable) = seekable {
            ready = ready.with_seekable(seekable);
        }
        Some(
            StatePayload::new()
                .with_playback_barrier_v1(PlaybackBarrierStateExtension::new().with_ready(ready)),
        )
    }

    /// Builds a StartedAck only after the caller has observed position
    /// advancement for the exact generation and revision in the retained
    /// CommitStart. Command acceptance alone must pass `position_advancing`
    /// as false and therefore cannot produce an acknowledgement.
    pub fn playback_barrier_started_observation(
        &self,
        media_generation: u64,
        state_revision: u64,
        observed_position: f64,
        position_advancing: bool,
        observed_at: Option<f64>,
    ) -> Option<StatePayload> {
        if !position_advancing
            || !observed_position.is_finite()
            || observed_position < 0.0
            || observed_at.is_some_and(|value| !value.is_finite())
        {
            return None;
        }
        let commit = self.playback_barrier_active_commit()?;
        if commit.media_generation != media_generation || commit.state_revision != state_revision {
            return None;
        }
        let mut started =
            StartedAckPayload::new(media_generation, state_revision, observed_position);
        if let Some(observed_at) = observed_at {
            started = started.with_observed_at(observed_at);
        }
        Some(
            StatePayload::new().with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new().with_started(started),
            ),
        )
    }

    /// Builds a deduplicated ongoing transport observation for the exact
    /// generation/revision accepted by the server's active buffering policy.
    /// The caller should queue the returned State through the connection-
    /// scoped State effect path.
    pub fn playback_barrier_transport_observation(
        &mut self,
        media_generation: u64,
        state_revision: Option<u64>,
        buffering: bool,
        buffered_seconds: Option<f64>,
        observed_at: Option<f64>,
    ) -> Option<StatePayload> {
        if buffered_seconds.is_some_and(|value| !value.is_finite() || value < 0.0)
            || observed_at.is_some_and(|value| !value.is_finite())
        {
            return None;
        }
        let policy = self.playback_barrier_buffering_policy()?;
        if media_generation == 0
            || policy.media_generation != media_generation
            || policy.state_revision != state_revision
            || self
                .playback_barrier
                .prepare
                .as_ref()
                .is_some_and(|prepare| prepare.media_generation > media_generation)
        {
            return None;
        }

        let mut transport = TransportBufferingReportPayload::new(media_generation, buffering);
        if let Some(state_revision) = state_revision {
            transport = transport.with_state_revision(state_revision);
        }
        if let Some(buffered_seconds) = buffered_seconds {
            transport = transport.with_buffered_seconds(buffered_seconds);
        }
        if let Some(observed_at) = observed_at {
            transport = transport.with_observed_at(observed_at);
        }
        if self.playback_barrier.last_transport_observation.as_ref() == Some(&transport) {
            return None;
        }
        self.playback_barrier.last_transport_observation = Some(transport.clone());
        Some(StatePayload::new().with_playback_barrier_v1(
            PlaybackBarrierStateExtension::new().with_transport(transport),
        ))
    }

    pub(super) fn reset_playback_barrier(&mut self) {
        self.playback_barrier = ClientPlaybackBarrierState::default();
    }

    pub(super) fn apply_playback_barrier_extension(
        &mut self,
        extension: PlaybackBarrierSetExtension,
    ) {
        if !self.playback_barrier_v1_negotiated() {
            return;
        }

        if let Some(prepare) = extension.prepare {
            self.apply_playback_barrier_prepare(prepare);
        }
        if let Some(commit) = extension.commit {
            self.apply_playback_barrier_commit(commit);
        }
        if let Some(status) = extension.status {
            self.apply_playback_barrier_status(status);
        }
        if let Some(policy) = extension.buffering_policy {
            self.apply_room_buffering_policy(policy);
        }
        if let Some(status) = extension.buffering_status {
            self.apply_room_buffering_status(status);
        }
    }

    fn apply_playback_barrier_prepare(&mut self, prepare: PrepareMediaPayload) {
        if prepare.media_generation == 0
            || prepare.request_nonce == 0
            || prepare.load_intent == MediaLoadIntent::TransportRefresh
            || prepare.logical_media_id.trim().is_empty()
            || !prepare.target_position.is_finite()
            || prepare.target_position < 0.0
            || !valid_prepare_quorum(&prepare)
        {
            return;
        }

        match self
            .playback_barrier
            .prepare
            .as_ref()
            .map(|current| prepare.media_generation.cmp(&current.media_generation))
        {
            Some(std::cmp::Ordering::Less) => {}
            Some(std::cmp::Ordering::Equal) => {
                self.playback_barrier.prepare = Some(prepare);
            }
            Some(std::cmp::Ordering::Greater) | None => {
                self.playback_barrier.prepare = Some(prepare);
                self.playback_barrier.commit = None;
                self.playback_barrier.status = None;
            }
        }
    }

    fn apply_playback_barrier_commit(&mut self, commit: CommitStartPayload) {
        let Some(active_generation) = self
            .playback_barrier
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation)
        else {
            return;
        };
        if commit.media_generation != active_generation
            || commit.state_revision == 0
            || !commit.anchor_position.is_finite()
            || commit.anchor_position < 0.0
            || !commit.anchor_server_time.is_finite()
            || !commit.started_deadline.is_finite()
            || commit.start_at.is_some_and(|value| !value.is_finite())
        {
            return;
        }
        if self
            .playback_barrier
            .commit
            .as_ref()
            .is_some_and(|current| current.state_revision > commit.state_revision)
        {
            return;
        }

        let revision_advanced = self
            .playback_barrier
            .commit
            .as_ref()
            .is_some_and(|current| current.state_revision < commit.state_revision);
        self.playback_barrier.commit = Some(commit);
        if revision_advanced {
            self.playback_barrier.status = None;
        }
    }

    fn apply_playback_barrier_status(&mut self, status: PlaybackBarrierStatusPayload) {
        let Some(active_generation) = self
            .playback_barrier
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation)
        else {
            return;
        };
        if status.media_generation != active_generation || !status.deadline.is_finite() {
            return;
        }

        match self.playback_barrier.commit.as_ref() {
            Some(commit) if status.state_revision != Some(commit.state_revision) => return,
            None if status.state_revision.is_some() => return,
            _ => {}
        }
        if self
            .playback_barrier
            .status
            .as_ref()
            .is_some_and(|current| {
                current.media_generation == status.media_generation
                    && current.state_revision == status.state_revision
                    && !playback_barrier_status_transition_allowed(current.phase, status.phase)
            })
        {
            return;
        }
        self.playback_barrier.status = Some(status);
    }

    fn apply_room_buffering_policy(&mut self, policy: RoomBufferingPolicyPayload) {
        if !valid_room_buffering_policy(&policy) {
            return;
        }
        if self
            .playback_barrier
            .buffering_policy
            .as_ref()
            .is_some_and(|current| room_buffering_identity_cmp(&policy, current).is_lt())
        {
            return;
        }

        if self.playback_barrier.buffering_policy.as_ref() != Some(&policy) {
            self.playback_barrier.buffering_status = None;
        }
        // A full policy is also the server's authoritative snapshot after a
        // join, reconnect, room switch, or capability upgrade. Even when its
        // identity is unchanged, the server has no report for this transport
        // yet, so allow the runtime to publish its current buffering state.
        self.playback_barrier.last_transport_observation = None;
        self.playback_barrier.buffering_report_epoch = self
            .playback_barrier
            .buffering_report_epoch
            .wrapping_add(1)
            .max(1);
        self.playback_barrier.buffering_policy = Some(policy);
    }

    fn apply_room_buffering_status(&mut self, status: RoomBufferingStatusPayload) {
        if !valid_room_buffering_status(&status) {
            return;
        }
        if self
            .playback_barrier
            .buffering_policy
            .as_ref()
            .is_some_and(|current| room_buffering_identity_cmp(&status.config, current).is_lt())
        {
            return;
        }

        let policy_changed =
            self.playback_barrier.buffering_policy.as_ref() != Some(&status.config);
        if policy_changed {
            self.playback_barrier.last_transport_observation = None;
        }
        self.playback_barrier.buffering_policy = Some(status.config.clone());
        self.playback_barrier.buffering_status = Some(status);
    }
}

fn playback_barrier_status_transition_allowed(
    current: PlaybackBarrierPhase,
    incoming: PlaybackBarrierPhase,
) -> bool {
    current == incoming
        || matches!(
            (current, incoming),
            (
                PlaybackBarrierPhase::Preparing,
                PlaybackBarrierPhase::Committed
                    | PlaybackBarrierPhase::AwaitingDecision
                    | PlaybackBarrierPhase::Complete
                    | PlaybackBarrierPhase::Degraded
            ) | (
                PlaybackBarrierPhase::Committed,
                PlaybackBarrierPhase::Complete | PlaybackBarrierPhase::Degraded
            ) | (
                PlaybackBarrierPhase::AwaitingDecision,
                PlaybackBarrierPhase::Degraded
            )
        )
}

fn valid_room_buffering_policy(policy: &RoomBufferingPolicyPayload) -> bool {
    if policy.media_generation == 0 || policy.state_revision == Some(0) {
        return false;
    }
    match policy.policy {
        RoomBufferingPolicy::Quorum => {
            if !policy
                .quorum_percent
                .is_some_and(|percent| (1..=100).contains(&percent))
            {
                return false;
            }
        }
        RoomBufferingPolicy::Independent
        | RoomBufferingPolicy::PauseController
        | RoomBufferingPolicy::PauseAnyEligible => {
            if policy.quorum_percent.is_some() {
                return false;
            }
        }
    }
    policy
        .max_pause_ms
        .is_none_or(|milliseconds| milliseconds != 0)
}

fn valid_room_buffering_status(status: &RoomBufferingStatusPayload) -> bool {
    let expected_required = match status.config.policy {
        RoomBufferingPolicy::Independent => 0,
        RoomBufferingPolicy::PauseController | RoomBufferingPolicy::PauseAnyEligible => {
            u32::from(status.eligible_clients > 0)
        }
        RoomBufferingPolicy::Quorum => {
            let percent = status.config.quorum_percent.unwrap_or_default();
            if status.eligible_clients == 0 {
                0
            } else {
                status
                    .eligible_clients
                    .saturating_mul(percent)
                    .saturating_add(99)
                    / 100
            }
        }
    };
    valid_room_buffering_policy(&status.config)
        && status
            .pause_deadline
            .is_none_or(|deadline| deadline.is_finite())
        && status.required_buffering_clients == expected_required
        && status.buffering_clients.len() <= status.eligible_clients as usize
        && matches!(
            (status.config.policy, status.phase),
            (
                RoomBufferingPolicy::Independent,
                RoomBufferingPhase::Independent
            ) | (
                RoomBufferingPolicy::PauseController
                    | RoomBufferingPolicy::PauseAnyEligible
                    | RoomBufferingPolicy::Quorum,
                RoomBufferingPhase::Monitoring
                    | RoomBufferingPhase::DebouncingPause
                    | RoomBufferingPhase::Paused
                    | RoomBufferingPhase::DebouncingResume
                    | RoomBufferingPhase::FailOpen
            )
        )
}

fn valid_prepare_quorum(prepare: &PrepareMediaPayload) -> bool {
    match prepare.policy {
        sorotte_protocol::PlaybackBarrierPolicy::Quorum => {
            prepare.quorum.is_some_and(|quorum| quorum > 0)
                && prepare
                    .quorum_percent
                    .is_none_or(|percent| (1..=100).contains(&percent))
        }
        sorotte_protocol::PlaybackBarrierPolicy::AllEligible
        | sorotte_protocol::PlaybackBarrierPolicy::Controller => {
            prepare.quorum.is_none() && prepare.quorum_percent.is_none()
        }
    }
}

fn room_buffering_identity_cmp(
    incoming: &RoomBufferingPolicyPayload,
    current: &RoomBufferingPolicyPayload,
) -> std::cmp::Ordering {
    incoming
        .media_generation
        .cmp(&current.media_generation)
        .then_with(|| match (incoming.state_revision, current.state_revision) {
            (Some(incoming), Some(current)) => incoming.cmp(&current),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        })
}
