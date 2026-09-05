//! Stream recovery owns interrupted-network evidence, attempt counts and the
//! cache-progress watchdog. Inputs are current physical load identity, accepted
//! positions and cache/seek observations; outputs are bounded same-generation
//! reload attempts. New attachment/load, seek, progress or terminal completion
//! resets the corresponding evidence; it never advances a shared playlist.
use super::*;

impl MpvAdapter {
    pub(super) fn interrupted_network_stream_recovery_load_command(
        &self,
        path: &str,
        resume_position_seconds: f64,
    ) -> Value {
        let mut options = self.network_media_options_map();
        options.insert(
            "start".to_owned(),
            Value::String(resume_position_seconds.to_string()),
        );
        json!([
            MPV_COMMAND_LOADFILE,
            path,
            MPV_LOADFILE_REPLACE,
            -1,
            Value::Object(options)
        ])
    }

    pub(super) fn observe_interrupted_network_stream_recovery_progress(
        &mut self,
        position_seconds: f64,
    ) {
        let generation = self.observation_media_generation();
        if let Some(recovery) = self
            .stream_recovery
            .interrupted_network_stream_recovery
            .as_mut()
            .filter(|recovery| {
                Some(recovery.media_generation) == generation
                    && position_seconds
                        >= recovery.resume_position_seconds
                            + INTERRUPTED_NETWORK_STREAM_RECOVERY_PROGRESS_SECONDS
            })
        {
            recovery.resume_position_seconds = position_seconds;
            recovery.consecutive_attempts = 0;
        }
    }

    pub(super) fn refresh_network_stream_recovery_evidence(&mut self) {
        let attachment_epoch = self.lifecycle_epoch();
        let Some(active_attempt) = self.player_lifecycle.active_attempt().cloned() else {
            self.stream_recovery.network_stream_recovery_evidence = None;
            return;
        };
        let media_generation = active_attempt.media_generation;
        let identity_matches = self
            .stream_recovery
            .network_stream_recovery_evidence
            .as_ref()
            .is_some_and(|evidence| {
                evidence.attachment_epoch == attachment_epoch
                    && evidence.media_generation == media_generation
                    && evidence.load_attempt_id == active_attempt.id
            });
        if !identity_matches {
            self.stream_recovery.network_stream_recovery_evidence = None;
        }
        if active_attempt.attachment_epoch != attachment_epoch
            || active_attempt.state.is_terminal()
            || active_attempt.superseded_by.is_some()
        {
            self.stream_recovery.network_stream_recovery_evidence = None;
            return;
        }
        if self.timeline_kind == PlayerTimelineKind::SlidingLive
            || (self.ytdl_is_live
                && self.ytdl_is_live_metadata_generation == Some(media_generation))
        {
            self.stream_recovery.network_stream_recovery_evidence = None;
            return;
        }
        if !self.active_file_loaded {
            return;
        }
        if self.observed_state.seeking == Some(true) {
            return;
        }

        let position_seconds = self
            .observed_state
            .position_seconds
            .filter(|position| position.is_finite() && *position >= 0.0);
        if let Some(position_seconds) = position_seconds
            && let Some(evidence) = self
                .stream_recovery
                .network_stream_recovery_evidence
                .as_mut()
                .filter(|evidence| {
                    evidence.attachment_epoch == attachment_epoch
                        && evidence.media_generation == media_generation
                        && evidence.load_attempt_id == active_attempt.id
                })
        {
            evidence.position_seconds = position_seconds;
        }

        let Some(path) = self.current_path.clone() else {
            // mpv commonly clears path immediately before end-file. Keep the
            // last coherent evidence until that causal terminal is classified.
            return;
        };
        if !uses_network_media_options(&path) {
            self.stream_recovery.network_stream_recovery_evidence = None;
            return;
        }
        let Some(duration_seconds) = self
            .observed_state
            .duration_seconds
            .filter(|duration| duration.is_finite() && *duration > 0.0)
        else {
            return;
        };
        let Some(position_seconds) = position_seconds else {
            return;
        };
        self.stream_recovery.network_stream_recovery_evidence =
            Some(NetworkStreamRecoveryEvidence {
                attachment_epoch,
                media_generation,
                load_attempt_id: active_attempt.id,
                path,
                duration_seconds,
                position_seconds,
            });
    }

    pub(super) fn try_recover_interrupted_network_stream(
        &mut self,
        generation: PlayerMediaGeneration,
    ) -> bool {
        self.try_recover_network_stream_with_minimum_remaining(
            generation,
            INTERRUPTED_NETWORK_STREAM_MINIMUM_REMAINING_SECONDS,
        )
    }

    pub(super) fn observe_network_cache_pause_for_recovery(&mut self, paused_for_cache: bool) {
        if !paused_for_cache {
            self.stream_recovery.network_cache_stall = None;
            return;
        }
        let Some(media_generation) = self.observation_media_generation() else {
            return;
        };
        let is_recoverable_network_rebuffer = self.active_file_loaded
            && self.active_generation_has_restarted
            && self.network_cache_stall_is_not_known_live(media_generation)
            && self.observed_state.seeking != Some(true)
            && self
                .current_path
                .as_deref()
                .is_some_and(uses_network_media_options);
        if !is_recoverable_network_rebuffer {
            return;
        }
        if self
            .stream_recovery
            .network_cache_stall
            .is_none_or(|stall| stall.media_generation != media_generation)
        {
            self.stream_recovery.network_cache_stall = Some(NetworkCacheStall {
                media_generation,
                last_progress_at: Instant::now(),
                last_sample: NetworkCacheProgressSample::from_observed_state(&self.observed_state),
            });
        }
    }

    pub(super) fn network_cache_stall_recovery_delay(&self) -> Duration {
        let configured_wait = self
            .network_options
            .network_media_options
            .get("cache-pause-wait")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .and_then(|value| Duration::try_from_secs_f64(value).ok())
            .unwrap_or_default();
        NETWORK_CACHE_STALL_RECOVERY_DELAY.max(
            configured_wait
                .checked_add(NETWORK_CACHE_STALL_RECOVERY_MARGIN)
                .unwrap_or(Duration::MAX),
        )
    }

    pub(super) fn network_cache_stall_is_not_known_live(
        &self,
        media_generation: PlayerMediaGeneration,
    ) -> bool {
        self.timeline_kind != PlayerTimelineKind::SlidingLive
            && !(self.ytdl_is_live
                && self.ytdl_is_live_metadata_generation == Some(media_generation))
    }

    pub(super) fn maintain_network_cache_stall_recovery(&mut self) {
        let Some(stall) = self.stream_recovery.network_cache_stall else {
            return;
        };
        let still_stalled = self.observation_media_generation() == Some(stall.media_generation)
            && self.active_file_loaded
            && self.active_generation_has_restarted
            && self.network_cache_stall_is_not_known_live(stall.media_generation)
            && self.observed_state.paused_for_cache == Some(true)
            && self.observed_state.seeking != Some(true)
            && self.observed_state.eof_reached != Some(true)
            && self.observed_state.cache_eof != Some(true)
            && self
                .current_path
                .as_deref()
                .is_some_and(uses_network_media_options);
        if !still_stalled {
            self.stream_recovery.network_cache_stall = None;
            return;
        }

        let sample = NetworkCacheProgressSample::from_observed_state(&self.observed_state);
        if let Some(active_stall) = self
            .stream_recovery
            .network_cache_stall
            .as_mut()
            .filter(|active| active.media_generation == stall.media_generation)
        {
            if sample.made_progress_since(active_stall.last_sample) {
                active_stall.last_progress_at = Instant::now();
                active_stall.last_sample = sample;
                return;
            }
            active_stall.last_sample = sample;
        }
        if stall.last_progress_at.elapsed() < self.network_cache_stall_recovery_delay() {
            return;
        }

        if self.try_recover_stalled_network_stream(stall.media_generation) {
            self.stream_recovery.network_cache_stall = None;
        } else if let Some(active_stall) = self
            .stream_recovery
            .network_cache_stall
            .as_mut()
            .filter(|active| active.media_generation == stall.media_generation)
        {
            // A rejected recovery stays bounded and backs off for another
            // watchdog interval instead of spinning in every runtime pump.
            active_stall.last_progress_at = Instant::now();
        }
    }

    /// Returns the number of bounded same-generation network reload attempts
    /// made since the most recent logical media load.
    pub fn network_stream_recovery_attempt_count(&self) -> usize {
        self.stream_recovery
            .interrupted_network_stream_recovery
            .map_or(0, |recovery| recovery.total_attempts)
    }

    pub(super) fn invalidate_network_stream_recovery_position_for_seek(&mut self) {
        // A seek makes the previously observed time-pos causally stale. Keep
        // recovery disabled until mpv publishes a fresh post-seek time-pos and
        // leaves its seeking state.
        self.stream_recovery.network_stream_recovery_evidence = None;
        self.observed_state.position_seconds = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct InterruptedNetworkStreamRecovery {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) latest_attempt_id: LoadAttemptId,
    pub(super) resume_position_seconds: f64,
    pub(super) consecutive_attempts: usize,
    pub(super) total_attempts: usize,
}

#[derive(Clone, PartialEq)]
pub(super) struct NetworkStreamRecoveryEvidence {
    pub(super) attachment_epoch: PlayerAttachmentEpoch,
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) load_attempt_id: LoadAttemptId,
    pub(super) path: String,
    pub(super) duration_seconds: f64,
    pub(super) position_seconds: f64,
}

impl fmt::Debug for NetworkStreamRecoveryEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkStreamRecoveryEvidence")
            .field("attachment_epoch", &self.attachment_epoch)
            .field("media_generation", &self.media_generation)
            .field("load_attempt_id", &self.load_attempt_id)
            .field("path", &sorotte_secret::REDACTED_SECRET)
            .field("duration_seconds", &self.duration_seconds)
            .field("position_seconds", &self.position_seconds)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NetworkCacheStall {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) last_progress_at: Instant,
    pub(super) last_sample: NetworkCacheProgressSample,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct NetworkCacheProgressSample {
    pub(super) position_seconds: Option<f64>,
    pub(super) buffered_ahead_seconds: Option<f64>,
    pub(super) buffered_ahead_bytes: Option<u64>,
    pub(super) cache_reader_position_seconds: Option<f64>,
    pub(super) cache_end_seconds: Option<f64>,
}

impl NetworkCacheProgressSample {
    pub(super) fn from_observed_state(state: &MpvObservedState) -> Self {
        Self {
            position_seconds: state.position_seconds,
            buffered_ahead_seconds: state.buffered_ahead_seconds,
            buffered_ahead_bytes: state.buffered_ahead_bytes,
            cache_reader_position_seconds: state.cache_reader_position_seconds,
            cache_end_seconds: state.cache_end_seconds,
        }
    }

    pub(super) fn made_progress_since(self, previous: Self) -> bool {
        fn f64_increased(current: Option<f64>, previous: Option<f64>) -> bool {
            match (current, previous) {
                (Some(current), Some(previous)) => {
                    current > previous + PLAYBACK_ADVANCEMENT_EPSILON_SECONDS
                }
                (Some(current), None) => current > PLAYBACK_ADVANCEMENT_EPSILON_SECONDS,
                _ => false,
            }
        }

        fn u64_increased(current: Option<u64>, previous: Option<u64>) -> bool {
            match (current, previous) {
                (Some(current), Some(previous)) => current > previous,
                (Some(current), None) => current > 0,
                _ => false,
            }
        }

        f64_increased(self.position_seconds, previous.position_seconds)
            || f64_increased(self.buffered_ahead_seconds, previous.buffered_ahead_seconds)
            || u64_increased(self.buffered_ahead_bytes, previous.buffered_ahead_bytes)
            || f64_increased(
                self.cache_reader_position_seconds,
                previous.cache_reader_position_seconds,
            )
            || f64_increased(self.cache_end_seconds, previous.cache_end_seconds)
    }
}

#[derive(Default)]
pub(super) struct StreamRecoveryState {
    pub(super) interrupted_network_stream_recovery: Option<InterruptedNetworkStreamRecovery>,
    pub(super) network_stream_recovery_evidence: Option<NetworkStreamRecoveryEvidence>,
    pub(super) network_cache_stall: Option<NetworkCacheStall>,
}

impl MpvAdapter {
    pub(super) fn try_recover_stalled_network_stream(
        &mut self,
        generation: PlayerMediaGeneration,
    ) -> bool {
        // A sustained, progress-free cache pause is independent evidence that
        // the request is dead, including near the media tail.
        self.try_recover_network_stream_with_minimum_remaining(
            generation,
            PLAYBACK_ADVANCEMENT_EPSILON_SECONDS,
        )
    }

    pub(super) fn try_recover_network_stream_with_minimum_remaining(
        &mut self,
        generation: PlayerMediaGeneration,
        minimum_remaining_seconds: f64,
    ) -> bool {
        let Some(active_attempt) = self.player_lifecycle.active_attempt().cloned() else {
            return false;
        };
        if active_attempt.media_generation != generation
            || active_attempt.state.is_terminal()
            || active_attempt.superseded_by.is_some()
        {
            return false;
        }
        self.refresh_network_stream_recovery_evidence();
        let Some(evidence) = self
            .stream_recovery
            .network_stream_recovery_evidence
            .as_ref()
            .filter(|evidence| {
                evidence.attachment_epoch == self.lifecycle_epoch()
                    && evidence.media_generation == generation
                    && evidence.load_attempt_id == active_attempt.id
            })
            .cloned()
        else {
            return false;
        };
        let NetworkStreamRecoveryEvidence {
            path,
            duration_seconds,
            position_seconds,
            ..
        } = evidence;
        if duration_seconds - position_seconds <= minimum_remaining_seconds {
            return false;
        }

        let (consecutive_attempts, total_attempts) = self
            .stream_recovery
            .interrupted_network_stream_recovery
            .filter(|recovery| recovery.media_generation == generation)
            .map_or((1, 1), |recovery| {
                (
                    recovery.consecutive_attempts.saturating_add(1),
                    recovery.total_attempts.saturating_add(1),
                )
            });
        if consecutive_attempts > MAX_CONSECUTIVE_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
            || total_attempts > MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
        {
            return false;
        }

        let attachment_epoch = self.lifecycle_epoch();
        let baseline_playlist_entry_ids = self.capture_authoritative_playlist_baseline();
        let attempt_id =
            self.submit_lifecycle_load(None, generation, &path, baseline_playlist_entry_ids);
        self.stream_recovery.interrupted_network_stream_recovery =
            Some(InterruptedNetworkStreamRecovery {
                media_generation: generation,
                latest_attempt_id: attempt_id,
                resume_position_seconds: position_seconds,
                consecutive_attempts,
                total_attempts,
            });
        let command =
            self.interrupted_network_stream_recovery_load_command(&path, position_seconds);
        if self
            .send_ipc_command_if_attached_without_draining_events(command)
            .is_err()
        {
            self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch,
                attempt_id,
                failure: PlayerCommandFailureKind::TransportDisconnected,
            });
            return false;
        }

        self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch,
            attempt_id,
        });
        self.network_options.network_media_options_embedded_load =
            (!self.network_options.network_media_options.is_empty()).then_some(
                EmbeddedNetworkMediaOptions {
                    media_generation: generation,
                    requested_target: path,
                },
            );
        self.pending_transport_telemetry_updates.retain(|update| {
            update.media_generation != Some(generation)
                || (update.phase != Some(PlayerTransportPhase::Ended)
                    && update.phase != Some(PlayerTransportPhase::Failed)
                    && update.eof_reached != Some(true))
        });
        self.lifecycle_reconciliation_due = true;
        #[cfg(not(test))]
        self.reconcile_lifecycle_from_authority();
        self.drain_ipc_events_if_attached();
        true
    }
}
