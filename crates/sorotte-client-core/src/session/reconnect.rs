use super::*;

impl ClientSession {
    pub fn reset_sync_state_for_reconnect(&mut self) {
        self.reset_sync_state_for_reconnect_with_attempt(0);
    }

    pub(super) fn reset_sync_state_for_reconnect_with_attempt(&mut self, attempt: u32) {
        self.reset_playback_barrier();
        self.model.cancel_connection_scoped_playback_transactions();
        self.mark_readiness_v2_reconnect_pending();
        let (ready_snapshot, file_snapshot, controller_snapshot) = self
            .model
            .connection
            .username
            .as_deref()
            .and_then(|username| self.model.room.users.get(username))
            .map(|user_view| {
                let ready_snapshot = user_view.ready;
                let file_snapshot = user_view.file.clone();
                let controller_snapshot = Some(user_view.controller);
                (ready_snapshot, file_snapshot, controller_snapshot)
            })
            .unwrap_or((None, None, None));
        let preserved_ready_snapshot = self.model.reconnect.ready_restore_snapshot.take().or(self
            .model
            .reconnect
            .ready_restore_intent
            .take());
        let preserved_file_snapshot = self.model.reconnect.file_restore_snapshot.take().or(self
            .model
            .reconnect
            .file_restore_intent
            .take());
        let preserved_controller_snapshot = self.model.reconnect.controller_restore_snapshot.take();
        let preserved_playlist_snapshot = self
            .model
            .reconnect
            .playlist_restore_snapshot
            .take()
            .or(self.model.reconnect.playlist_restore_intent.take())
            .or(self.model.reconnect.playlist_restore_pending_ack.take());

        self.model.reconnect.ready_restore_snapshot = preserved_ready_snapshot.or(ready_snapshot);
        self.model.reconnect.ready_restore_intent = None;
        self.model.reconnect.file_restore_snapshot = preserved_file_snapshot.or(file_snapshot);
        self.model.reconnect.file_restore_intent = None;
        self.model.reconnect.controller_restore_snapshot =
            preserved_controller_snapshot.or(controller_snapshot);

        self.model.reconnect.playlist_restore_snapshot =
            preserved_playlist_snapshot.or_else(|| {
                self.current_room_playlist()
                    .and_then(Self::playlist_restore_intent_from_room_playlist)
            });
        self.model.reconnect.playlist_restore_intent = None;
        self.model.reconnect.playlist_restore_pending_ack = None;
        self.model.reconnect.connected_intent = false;
        self.clear_reconnect_state_restore_validation_state();
        self.pending_chat_notifications.clear();
        self.pending_controlled_room_creation_notifications.clear();
        self.pending_controller_auth_notifications.clear();
        self.pending_user_change_notifications.clear();
        self.model.controller.controlled_room_switch_intent = None;
        self.model.controller.pending_local_room_switch_target = None;
        self.model.controller.reidentify_intent = None;
        self.model.room.users.clear();
        self.model.room.participant_status_capabilities.clear();
        self.model.room.legacy_list_position_snapshots.clear();
        self.clear_participant_status_views();
        self.model.room.media_match_peer_tiers.clear();
        self.model.room.known_rooms.clear();
        self.model.room.domain = SyncDomain::default();
        self.model.playlist.rooms.clear();
        self.model.room.playstates.clear();
        self.model.room.playstate_transport_revisions.clear();
        self.model.room.playstate_receipt_sequences.clear();
        self.pending_playstate_transport_evidence = None;
        self.model.room.playstate_updated_at_seconds.clear();
        self.model
            .room
            .playstate_authority_changed_at_seconds
            .clear();
        self.model.playlist.pending = None;
        self.model.playlist.pending_remote_revision = 0;
        self.model.playlist.selection_revisions.clear();
        self.model.playlist.pending_selection_revision = 0;
        self.model.playlist.canonical_epochs.clear();
        self.model.playlist.pending_canonical_epoch = None;
        self.model.playlist.pending_local_change_echoes.clear();
        self.model.playlist.pending_local_index_echoes.clear();
        self.model.playlist.remote_revisions.clear();
        self.model.playlist.undo_snapshots.clear();
        self.model.playlist.shuffle_nonce = 0;
        self.reset_playlist_index_transition_tracking();
        self.model.playback.local_position = None;
        self.model.playback.local_paused = None;
        self.model.playback.local_playback_rate = None;
        self.model.playback.local_paused_for_cache = None;
        self.model.playback.local_cache_buffering_percent = None;
        self.model.playback.pending_cache_room_playstate_resync = false;
        self.model.playback.cache_recovery_observation_position = None;
        self.model
            .playback
            .cache_recovery_waiting_for_post_cache_position = false;
        self.model.playlist.last_seek_position_before_manual_seek = None;
        self.model.readiness.autoplay_timer_running = false;
        self.model.readiness.autoplay_time_left_seconds =
            self.model.readiness.config.autoplay_delay_seconds;
        self.model.playback.speed_changed = false;
        self.model.playback.speed_correction_rate = None;
        self.model.playback.behind_first_detected_at_seconds = None;
        self.model.playback.last_paused_on_leave_at_seconds = None;
        self.model.playback.last_advanced_at_seconds = None;
        self.model.playback.client_ignoring_on_the_fly = 0;
        self.model.playback.server_ignoring_on_the_fly = 0;
        self.mark_reconnecting(attempt);
        self.model.playback.last_rewound_at_seconds = None;

        if let (Some(username), Some(room_name)) = (
            self.model.connection.username.clone(),
            self.model.room.name.clone(),
        ) {
            self.set_user_room(&username, Some(room_name));
            // Pending V2 intent is presentation/outbox state, never a
            // canonical room-user projection. Preserve only the last server
            // snapshot while reconnecting; a fresh Hello/snapshot will then
            // confirm or replace it for the new transport.
            let preserved_v2_ready = self
                .model
                .readiness
                .canonical_snapshot
                .as_ref()
                .filter(|_| {
                    self.model.readiness.canonical_room.as_deref()
                        == self.model.room.name.as_deref()
                })
                .and_then(|snapshot| snapshot.participants.get(&username))
                .map(|participant| participant.room_ready);
            self.set_user_ready_state(&username, Some(preserved_v2_ready.unwrap_or(false)));
        }
    }

    pub fn reconcile_state_and_build_response(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
    ) -> StatePayload {
        self.reconcile_state_and_build_response_at(
            inbound_state,
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub(crate) fn reconcile_state_and_build_response_at(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
        received_at_seconds: f64,
    ) -> StatePayload {
        self.reconcile_state_and_build_response_at_with_pause_mutation_intent(
            inbound_state,
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
            received_at_seconds,
            Some(LocalPauseMutationIntent {
                paused: local_paused,
                base_transport_revision: self.current_room_transport_revision(),
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_state_and_build_response_at_with_pause_mutation_intent(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
        received_at_seconds: f64,
        local_pause_mutation_intent: Option<LocalPauseMutationIntent>,
    ) -> StatePayload {
        self.reconcile_normalized_state_and_build_response_with_local_state_change_override(
            normalize_client_state_payload(inbound_state),
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
            StateReconcileContext {
                local_state_change_global_playstate: None,
                local_pause_mutation_intent,
                received_at_seconds,
            },
        )
    }

    pub(crate) fn reconcile_ping_only_state_response(
        &mut self,
        mut inbound_state: ClientStateUpdate,
        client_latency_calculation: f64,
        client_rtt: f64,
        received_at_seconds: f64,
    ) -> StatePayload {
        self.apply_participant_status_update(
            inbound_state.participant_status_scope.take(),
            inbound_state.participant_status_snapshot.take(),
            std::mem::take(&mut inbound_state.participant_status_scope_invalid),
            received_at_seconds,
        );
        self.apply_inbound_ignore_counters(&inbound_state);

        let has_playstate_update = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some());
        let current_transport_revision = self.current_room_transport_revision();
        let transport_revision_is_rejected =
            inbound_state.playstate.as_ref().is_some_and(|playstate| {
                match (playstate.transport_revision, current_transport_revision) {
                    (Some(0), _) => true,
                    (Some(candidate), Some(current)) => candidate < current,
                    (None, Some(_)) => true,
                    _ => false,
                }
            });
        let revision_or_seek_edge = inbound_state.playstate.as_ref().is_some_and(|playstate| {
            playstate.transport_revision.is_some_and(|revision| {
                current_transport_revision != Some(revision) || playstate.do_seek == Some(true)
            })
        });
        let may_apply_playstate = has_playstate_update
            && self.model.playback.client_ignoring_on_the_fly == 0
            && !transport_revision_is_rejected;
        if revision_or_seek_edge
            && may_apply_playstate
            && let Some(playstate) = inbound_state.playstate.as_ref()
            && let Some(transport_revision) = playstate.transport_revision
        {
            self.stage_pending_playstate_transport_evidence(
                playstate,
                transport_revision,
                received_at_seconds,
            );
        }
        if may_apply_playstate {
            self.apply_state_at(inbound_state.clone(), Some(received_at_seconds));
        }

        let mut ping = PingPayload::new()
            .with_client_latency_calculation(client_latency_calculation)
            .with_client_rtt(client_rtt);
        if let Some(latency_calculation) = inbound_state
            .ping
            .as_ref()
            .and_then(|ping| ping.latency_calculation)
            && latency_calculation != 0.0
        {
            ping = ping.with_latency_calculation(latency_calculation);
        }

        let mut response = StatePayload::new().with_ping(ping);
        if self.model.playback.server_ignoring_on_the_fly != 0
            || self.model.playback.client_ignoring_on_the_fly != 0
        {
            let mut ignore = IgnoringOnTheFlyPayload::new();
            if self.model.playback.server_ignoring_on_the_fly != 0 {
                ignore = ignore.with_server(self.model.playback.server_ignoring_on_the_fly);
                self.model.playback.server_ignoring_on_the_fly = 0;
            }
            if self.model.playback.client_ignoring_on_the_fly != 0 {
                ignore = ignore.with_client(self.model.playback.client_ignoring_on_the_fly);
            }
            response.ignoring_on_the_fly = Some(ignore);
        }

        response
    }

    #[cfg(test)]
    pub(crate) fn reconcile_state_and_build_response_with_local_state_change_override(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
    ) -> StatePayload {
        self.reconcile_normalized_state_and_build_response_with_local_state_change_override(
            normalize_client_state_payload(inbound_state),
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
            StateReconcileContext {
                local_state_change_global_playstate,
                local_pause_mutation_intent: Some(LocalPauseMutationIntent {
                    paused: local_paused,
                    base_transport_revision: self.current_room_transport_revision(),
                }),
                received_at_seconds: unix_wall_clock_time_seconds_legacy_compatible(),
            },
        )
    }

    pub(crate) fn reconcile_normalized_state_and_build_response_with_local_state_change_override(
        &mut self,
        mut inbound_state: ClientStateUpdate,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
        context: StateReconcileContext,
    ) -> StatePayload {
        let StateReconcileContext {
            local_state_change_global_playstate,
            local_pause_mutation_intent,
            received_at_seconds,
        } = context;
        let inbound_transport_revision = inbound_state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.transport_revision);
        self.apply_participant_status_update(
            inbound_state.participant_status_scope.take(),
            inbound_state.participant_status_snapshot.take(),
            std::mem::take(&mut inbound_state.participant_status_scope_invalid),
            received_at_seconds,
        );
        self.apply_inbound_ignore_counters(&inbound_state);

        let had_global_playstate = self.has_global_playstate();
        let current_transport_revision = self.current_room_transport_revision();
        let has_playstate_update = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some());
        let transport_revision_is_rejected =
            inbound_state.playstate.as_ref().is_some_and(|playstate| {
                match (playstate.transport_revision, current_transport_revision) {
                    (Some(0), _) => true,
                    (Some(candidate), Some(current)) => candidate < current,
                    (None, Some(_)) => true,
                    _ => false,
                }
            });
        let revision_or_seek_edge = inbound_state.playstate.as_ref().is_some_and(|playstate| {
            playstate.transport_revision.is_some_and(|revision| {
                current_transport_revision != Some(revision) || playstate.do_seek == Some(true)
            })
        });
        let local_intent_supersedes_first_transport_baseline = current_transport_revision.is_none()
            && !transport_revision_is_rejected
            && inbound_state
                .playstate
                .as_ref()
                .is_some_and(|playstate| playstate.do_seek != Some(true))
            && local_pause_mutation_intent.is_some_and(|intent| {
                intent.base_transport_revision == inbound_transport_revision
                    && inbound_state
                        .playstate
                        .as_ref()
                        .and_then(|playstate| playstate.paused)
                        != Some(intent.paused)
            });
        let pending_transport_evidence_superseded_by_local_intent = self
            .pending_playstate_transport_evidence
            .as_ref()
            .zip(local_pause_mutation_intent)
            .is_some_and(|(pending, intent)| {
                intent.base_transport_revision == Some(pending.transport_revision)
                    && intent.paused != pending.paused
            });
        if pending_transport_evidence_superseded_by_local_intent {
            // A user command based on this exact canonical revision is newer
            // than the still-pending player proof for that revision. Waiting
            // for the superseded pause state would deadlock the new command's
            // State response forever.
            self.pending_playstate_transport_evidence = None;
        }
        let pending_transport_evidence_satisfied = self
            .pending_playstate_transport_evidence
            .as_ref()
            .is_some_and(|pending| {
                self.model.room.name.as_deref() == Some(pending.room.as_str())
                    && current_transport_revision == Some(pending.transport_revision)
                    && local_paused == pending.paused
                    && pending.seek_position_seconds.is_none_or(|position| {
                        let elapsed = if !pending.paused
                            && received_at_seconds.is_finite()
                            && pending.authority_observed_at_seconds.is_finite()
                            && received_at_seconds >= pending.authority_observed_at_seconds
                        {
                            received_at_seconds - pending.authority_observed_at_seconds
                        } else {
                            0.0
                        };
                        local_position.is_finite()
                            && (local_position - (position + elapsed)).abs()
                                <= SEEK_THRESHOLD_SECONDS
                    })
            });
        if pending_transport_evidence_satisfied {
            self.pending_playstate_transport_evidence = None;
        }
        let may_apply_playstate = has_playstate_update
            && self.model.playback.client_ignoring_on_the_fly == 0
            && !transport_revision_is_rejected;
        let stage_transport_evidence = revision_or_seek_edge
            && !local_intent_supersedes_first_transport_baseline
            && may_apply_playstate
            && inbound_transport_revision.is_some();
        if stage_transport_evidence {
            let playstate = inbound_state
                .playstate
                .as_ref()
                .expect("a complete transport edge must retain playstate");
            self.stage_pending_playstate_transport_evidence(
                playstate,
                inbound_transport_revision.expect("a staged transport edge must carry a revision"),
                received_at_seconds,
            );
        }
        let withhold_playstate_until_post_revision_player_evidence = transport_revision_is_rejected
            || (revision_or_seek_edge && !local_intent_supersedes_first_transport_baseline)
            || self.pending_playstate_transport_evidence.is_some();
        let initial_playstate_is_local_echo = inbound_state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.set_by.as_deref())
            .zip(self.model.connection.username.as_deref())
            .is_some_and(|(set_by, username)| set_by == username);
        let initial_remote_baseline = (!had_global_playstate
            && has_playstate_update
            && local_pause_mutation_intent.is_none()
            && !initial_playstate_is_local_echo)
            .then(|| {
                local_state_change_global_playstate
                    .clone()
                    .or_else(|| {
                        inbound_state
                            .playstate
                            .as_ref()
                            .map(|playstate| RoomPlaystateView {
                                position: playstate.position,
                                paused: playstate.paused,
                                do_seek: Some(playstate.do_seek.unwrap_or(false)),
                                set_by: playstate.set_by.clone(),
                            })
                    })
                    .expect("a complete initial playstate must produce a room baseline")
            });
        if has_playstate_update && self.model.playback.client_ignoring_on_the_fly == 0 {
            self.apply_state_at(inbound_state.clone(), Some(received_at_seconds));
        }

        let mut response = StatePayload::new();
        let has_global_playstate = self.has_global_playstate();
        let client_ignore_not_set = self.model.playback.client_ignoring_on_the_fly == 0
            || self.model.playback.server_ignoring_on_the_fly != 0;
        let canonical_paused = local_state_change_global_playstate
            .as_ref()
            .and_then(|playstate| playstate.paused)
            .or_else(|| {
                self.current_room_playstate_at(received_at_seconds)
                    .and_then(|playstate| playstate.paused)
            });

        let mut state_change = false;
        if has_global_playstate
            && client_ignore_not_set
            && !withhold_playstate_until_post_revision_player_evidence
        {
            let reconciled_local_position = initial_remote_baseline
                .as_ref()
                .and_then(|playstate| playstate.position)
                .unwrap_or(local_position);
            let reconciled_local_paused = initial_remote_baseline
                .as_ref()
                .and_then(|playstate| playstate.paused)
                .or_else(|| {
                    local_pause_mutation_intent
                        .is_none()
                        .then_some(canonical_paused)
                        .flatten()
                })
                .unwrap_or_else(|| {
                    local_pause_mutation_intent
                        .map(|intent| intent.paused)
                        .unwrap_or(local_paused)
                });
            let (pause_change, seeked) = self
                .determine_local_state_change_with_global_playstate_override_at(
                    reconciled_local_paused,
                    reconciled_local_position,
                    local_state_change_global_playstate,
                    received_at_seconds,
                );

            let mut playstate = PlaystatePayload::new()
                .with_position(reconciled_local_position)
                .with_paused(reconciled_local_paused);
            if seeked {
                playstate = playstate.with_do_seek(true);
            }
            if let Some(transport_revision) =
                inbound_transport_revision.or_else(|| self.current_room_transport_revision())
            {
                playstate = playstate.with_transport_revision(transport_revision);
            }
            response.playstate = Some(playstate);
            state_change = pause_change || seeked;
        }

        // The response projection above may echo canonical authority while a
        // late joiner or a just-reconnected player is still physically behind.
        // Keep the local model tied to the sampled player, otherwise the
        // playback coordinator sees a fabricated convergence and never issues
        // the Pause/Play/Seek required to catch the player up.
        self.model.playback.local_position = Some(local_position);
        self.model.playback.local_paused = Some(local_paused);

        let mut ping = PingPayload::new()
            .with_client_latency_calculation(client_latency_calculation)
            .with_client_rtt(client_rtt);
        if let Some(latency_calculation) = inbound_state
            .ping
            .as_ref()
            .and_then(|ping| ping.latency_calculation)
            && latency_calculation != 0.0
        {
            ping = ping.with_latency_calculation(latency_calculation);
        }
        response.ping = Some(ping);

        if state_change {
            self.model.playback.client_ignoring_on_the_fly = self
                .model
                .playback
                .client_ignoring_on_the_fly
                .saturating_add(1);
        }

        if self.model.playback.server_ignoring_on_the_fly != 0
            || self.model.playback.client_ignoring_on_the_fly != 0
        {
            let mut ignore = IgnoringOnTheFlyPayload::new();
            if self.model.playback.server_ignoring_on_the_fly != 0 {
                ignore = ignore.with_server(self.model.playback.server_ignoring_on_the_fly);
                self.model.playback.server_ignoring_on_the_fly = 0;
            }
            if self.model.playback.client_ignoring_on_the_fly != 0 {
                ignore = ignore.with_client(self.model.playback.client_ignoring_on_the_fly);
            }
            response.ignoring_on_the_fly = Some(ignore);
        }

        response
    }

    fn stage_pending_playstate_transport_evidence(
        &mut self,
        playstate: &ClientPlaystate,
        transport_revision: u64,
        received_at_seconds: f64,
    ) {
        self.pending_playstate_transport_evidence = Some(PendingPlaystateTransportEvidence {
            room: self.model.room.name.clone().unwrap_or_default(),
            transport_revision,
            paused: playstate
                .paused
                .expect("a complete transport edge must carry paused"),
            seek_position_seconds: (playstate.do_seek == Some(true))
                .then_some(playstate.position)
                .flatten()
                .filter(|position| position.is_finite()),
            authority_observed_at_seconds: received_at_seconds,
        });
    }
}
