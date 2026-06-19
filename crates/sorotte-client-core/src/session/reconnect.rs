use super::*;

impl ClientSession {
    pub fn reset_sync_state_for_reconnect(&mut self) {
        let (ready_snapshot, file_snapshot, controller_snapshot) = self
            .username
            .as_deref()
            .and_then(|username| self.user_views.get(username))
            .map(|user_view| {
                let ready_snapshot = user_view.ready;
                let file_snapshot = if user_view.has_file {
                    Self::file_payload_from_user_view(user_view)
                } else {
                    None
                };
                let controller_snapshot = Some(user_view.controller);
                (ready_snapshot, file_snapshot, controller_snapshot)
            })
            .unwrap_or((None, None, None));
        let preserved_ready_snapshot = self
            .reconnect_ready_restore_snapshot
            .take()
            .or(self.reconnect_ready_restore_intent.take());
        let preserved_file_snapshot = self
            .reconnect_file_restore_snapshot
            .take()
            .or(self.reconnect_file_restore_intent.take());
        let preserved_controller_snapshot = self.reconnect_controller_restore_snapshot.take();
        let preserved_playlist_snapshot = self
            .reconnect_playlist_restore_snapshot
            .take()
            .or(self.reconnect_playlist_restore_intent.take());

        self.reconnect_ready_restore_snapshot = preserved_ready_snapshot.or(ready_snapshot);
        self.reconnect_ready_restore_intent = None;
        self.reconnect_file_restore_snapshot = preserved_file_snapshot.or(file_snapshot);
        self.reconnect_file_restore_intent = None;
        self.reconnect_controller_restore_snapshot =
            preserved_controller_snapshot.or(controller_snapshot);

        self.reconnect_playlist_restore_snapshot = preserved_playlist_snapshot.or_else(|| {
            self.current_room_playlist()
                .and_then(Self::playlist_restore_intent_from_room_playlist)
        });
        self.reconnect_playlist_restore_intent = None;
        self.reconnect_connected_intent = false;
        self.clear_reconnect_state_restore_validation_state();
        self.pending_chat_notifications.clear();
        self.pending_controlled_room_creation_notifications.clear();
        self.pending_controller_auth_notifications.clear();
        self.pending_user_change_notifications.clear();
        self.controlled_room_switch_intent = None;
        self.pending_local_room_switch_target = None;
        self.controller_reidentify_intent = None;
        self.user_views.clear();
        self.media_match_peer_tiers.clear();
        self.known_rooms.clear();
        self.domain = SyncDomain::default();
        self.room_playlists.clear();
        self.room_playstates.clear();
        self.room_playstate_updated_at_seconds.clear();
        self.pending_playlist = None;
        self.playlist_undo_snapshots.clear();
        self.playlist_shuffle_nonce = 0;
        self.reset_playlist_index_transition_tracking();
        self.local_position = None;
        self.local_paused = None;
        self.local_paused_for_cache = None;
        self.local_cache_buffering_percent = None;
        self.pending_cache_room_playstate_resync = false;
        self.last_seek_position_before_manual_seek = None;
        self.autoplay_timer_running = false;
        self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
        self.speed_changed = false;
        self.behind_first_detected_at_seconds = None;
        self.last_paused_on_leave_at_seconds = None;
        self.last_advanced_at_seconds = None;
        self.client_ignoring_on_the_fly = 0;
        self.server_ignoring_on_the_fly = 0;
        self.clear_server_feature_support_state();
        self.last_rewound_at_seconds = None;

        if let (Some(username), Some(room_name)) = (self.username.clone(), self.room.clone()) {
            self.set_user_room(&username, Some(room_name));
            self.set_user_ready_state(&username, Some(false));
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
        self.reconcile_state_and_build_response_with_local_state_change_override(
            inbound_state,
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
            None,
        )
    }

    pub(crate) fn reconcile_ping_only_state_response(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
    ) -> StatePayload {
        self.apply_inbound_ignore_counters(&inbound_state);

        let has_playstate_update = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some());
        if has_playstate_update && self.client_ignoring_on_the_fly == 0 {
            self.apply_state(inbound_state.clone());
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
        if self.server_ignoring_on_the_fly != 0 || self.client_ignoring_on_the_fly != 0 {
            let mut ignore = IgnoringOnTheFlyPayload::new();
            if self.server_ignoring_on_the_fly != 0 {
                ignore = ignore.with_server(self.server_ignoring_on_the_fly);
                self.server_ignoring_on_the_fly = 0;
            }
            if self.client_ignoring_on_the_fly != 0 {
                ignore = ignore.with_client(self.client_ignoring_on_the_fly);
            }
            response.ignoring_on_the_fly = Some(ignore);
        }

        response
    }

    pub(crate) fn reconcile_state_and_build_response_with_local_state_change_override(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
    ) -> StatePayload {
        self.apply_inbound_ignore_counters(&inbound_state);

        let has_playstate_update = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some());
        if has_playstate_update && self.client_ignoring_on_the_fly == 0 {
            self.apply_state(inbound_state.clone());
        }

        let mut response = StatePayload::new();
        let has_global_playstate = self.has_global_playstate();
        let client_ignore_not_set =
            self.client_ignoring_on_the_fly == 0 || self.server_ignoring_on_the_fly != 0;

        let mut state_change = false;
        if has_global_playstate && client_ignore_not_set {
            let (pause_change, seeked) = self
                .determine_local_state_change_with_global_playstate_override(
                    local_paused,
                    local_position,
                    local_state_change_global_playstate,
                );

            let mut playstate = PlaystatePayload::new()
                .with_position(local_position)
                .with_paused(local_paused);
            if seeked {
                playstate = playstate.with_do_seek(true);
            }
            response.playstate = Some(playstate);
            state_change = pause_change || seeked;
        }

        self.local_position = Some(local_position);
        self.local_paused = Some(local_paused);

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
            self.client_ignoring_on_the_fly = self.client_ignoring_on_the_fly.saturating_add(1);
        }

        if self.server_ignoring_on_the_fly != 0 || self.client_ignoring_on_the_fly != 0 {
            let mut ignore = IgnoringOnTheFlyPayload::new();
            if self.server_ignoring_on_the_fly != 0 {
                ignore = ignore.with_server(self.server_ignoring_on_the_fly);
                self.server_ignoring_on_the_fly = 0;
            }
            if self.client_ignoring_on_the_fly != 0 {
                ignore = ignore.with_client(self.client_ignoring_on_the_fly);
            }
            response.ignoring_on_the_fly = Some(ignore);
        }

        response
    }
}
