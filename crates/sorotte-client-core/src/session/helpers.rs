use super::*;

impl ClientSession {
    pub(super) fn merge_room_playstate(
        &mut self,
        room_name: String,
        playstate: ClientPlaystate,
        updated_at_seconds: f64,
    ) {
        let room_key = room_name.clone();
        let room_playstate = self.model.room.playstates.entry(room_name).or_default();
        if let Some(position) = playstate.position {
            room_playstate.position = Some(position);
        }
        if let Some(paused) = playstate.paused {
            room_playstate.paused = Some(paused);
        }
        room_playstate.do_seek = Some(playstate.do_seek.unwrap_or(false));
        room_playstate.set_by = playstate.set_by;
        self.model
            .room
            .playstate_updated_at_seconds
            .insert(room_key, updated_at_seconds);
    }

    pub(super) fn apply_inbound_ignore_counters(&mut self, state_payload: &ClientStateUpdate) {
        let Some(ignore) = state_payload.ignoring_on_the_fly.as_ref() else {
            return;
        };

        if let Some(server) = ignore.server {
            self.model.playback.server_ignoring_on_the_fly = server;
            self.model.playback.client_ignoring_on_the_fly = 0;
        } else if let Some(client) = ignore.client
            && client == self.model.playback.client_ignoring_on_the_fly
        {
            self.model.playback.client_ignoring_on_the_fly = 0;
        }
    }

    pub(super) fn has_global_playstate(&self) -> bool {
        self.current_room_playstate()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some())
    }

    pub(super) fn effective_local_paused_state(&self, now_seconds: f64) -> bool {
        if self.model.playback.local_paused_for_cache == Some(true) {
            return true;
        }

        self.model
            .playback
            .local_paused
            .or_else(|| {
                self.current_room_playstate_at(now_seconds)
                    .and_then(|playstate| playstate.paused)
            })
            .unwrap_or(true)
    }

    pub(super) fn shared_playlist_runtime_commands_allowed_legacy_compatible(&self) -> bool {
        self.is_active()
            && self.model.connection.username.is_some()
            && self.model.room.name.is_some()
            && self.server_shared_playlists_supported()
    }

    pub(super) fn apply_local_ready_state_optimistically(&mut self, ready: bool) {
        let Some(username) = self.model.connection.username.clone() else {
            return;
        };
        self.set_user_ready_state(&username, Some(ready));
    }

    pub(super) fn runtime_actions_for_local_pause_change(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.playback.local_paused_for_cache == Some(true) && !paused {
            return Vec::new();
        }

        let effective_paused = self.effective_local_paused_state(now_seconds);
        if effective_paused == paused {
            return Vec::new();
        }

        if self.model.connection.username.is_none() || !self.server_readiness_supported() {
            self.model.playback.local_paused = Some(paused);
            return vec![ClientRuntimeAction::SetPaused(paused)];
        }

        let local_can_control = self.local_can_control().unwrap_or(false);
        let is_playing_music = self.is_playing_music();
        let recently_advanced = self.recently_advanced(now_seconds);
        let global_paused = self
            .current_room_playstate_at(now_seconds)
            .and_then(|playstate| playstate.paused)
            .unwrap_or(true);

        if !local_can_control {
            self.model.playback.local_paused = Some(global_paused);
            let mut actions = Vec::new();
            if effective_paused != global_paused {
                actions.push(ClientRuntimeAction::SetPaused(global_paused));
            }
            if paused != global_paused
                && (!global_paused || recently_advanced)
                && !self.local_user_ready()
                && !self.recently_rewound(now_seconds, RECENT_REWIND_READINESS_SUPPRESSION_SECONDS)
            {
                self.apply_local_ready_state_optimistically(true);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if is_playing_music && recently_advanced {
            self.model.playback.local_paused = Some(paused);
            return vec![ClientRuntimeAction::SetPaused(paused)];
        }

        if paused {
            self.model.playback.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if self.local_user_ready() {
                self.apply_local_ready_state_optimistically(false);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: false,
                    manually_initiated: false,
                });
            }
            return actions;
        }

        let instaplay = self.instaplay_conditions_met(local_can_control, is_playing_music);
        if !instaplay {
            self.model.playback.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if !self.local_user_ready() {
                self.apply_local_ready_state_optimistically(true);
                actions.push(ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if let Some(last_paused_on_leave_at_seconds) =
            self.model.playback.last_paused_on_leave_at_seconds
            && now_seconds - last_paused_on_leave_at_seconds
                < self
                    .model
                    .readiness
                    .config
                    .last_paused_diff_threshold_seconds
        {
            self.model.playback.last_paused_on_leave_at_seconds = None;
            self.model.playback.local_paused = Some(false);
            return vec![ClientRuntimeAction::SetPaused(false)];
        }

        self.model.playback.local_paused = Some(false);
        let mut actions = vec![ClientRuntimeAction::SetPaused(false)];
        if !self.local_user_ready() {
            self.apply_local_ready_state_optimistically(true);
            actions.push(ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false,
            });
        }
        actions
    }

    #[cfg(test)]
    pub(super) fn determine_local_state_change(
        &self,
        local_paused: bool,
        local_position: f64,
    ) -> (bool, bool) {
        self.determine_local_state_change_with_global_playstate_override(
            local_paused,
            local_position,
            None,
        )
    }

    pub(super) fn determine_local_state_change_with_global_playstate_override(
        &self,
        local_paused: bool,
        local_position: f64,
        global_playstate_override: Option<RoomPlaystateView>,
    ) -> (bool, bool) {
        let global_playstate = global_playstate_override.or_else(|| {
            self.current_room_playstate_at(unix_wall_clock_time_seconds_legacy_compatible())
        });
        let global_paused = global_playstate
            .as_ref()
            .and_then(|playstate| playstate.paused)
            .unwrap_or(true);
        let global_position = global_playstate
            .as_ref()
            .and_then(|playstate| playstate.position)
            .unwrap_or(0.0);
        let player_paused = self.model.playback.local_paused.unwrap_or(global_paused);
        let player_position = self
            .model
            .playback
            .local_position
            .unwrap_or(global_position);

        let pause_change = player_paused != local_paused && global_paused != local_paused;
        let seeked = (player_position - local_position).abs() > SEEK_THRESHOLD_SECONDS
            && (global_position - local_position).abs() > SEEK_THRESHOLD_SECONDS;
        (pause_change, seeked)
    }

    pub(super) fn local_username_and_room(&self) -> Option<(&str, &str)> {
        let local_username = self.model.connection.username.as_deref()?;
        let local_room = self.model.room.name.as_deref()?;
        Some((local_username, local_room))
    }

    pub(super) fn current_room_has_other_users(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };

        self.model.room.users.iter().any(|(username, user_view)| {
            username != local_username && user_view.room.as_deref() == Some(local_room)
        })
    }

    pub(super) fn room_playstate_has_remote_authority(
        &self,
        playstate: &RoomPlaystateView,
    ) -> bool {
        match playstate.set_by.as_deref() {
            Some(set_by) => self.model.connection.username.as_deref() != Some(set_by),
            None => self.current_room_has_other_users(),
        }
    }

    pub(super) fn should_track_playlist_index_transition_for_room(
        &self,
        room_name: Option<&str>,
    ) -> bool {
        let tracked_room = self
            .model
            .controller
            .pending_local_room_switch_target
            .as_deref()
            .or(self.model.room.name.as_deref());
        matches!((tracked_room, room_name), (Some(tracked_room), Some(room_name)) if tracked_room == room_name)
    }

    pub(super) fn update_local_room(&mut self, room_name: String) {
        if self.model.room.name.as_deref() != Some(room_name.as_str()) {
            self.reset_playback_barrier();
            self.reset_playlist_index_transition_tracking();
            self.model.controller.pending_local_room_switch_target = None;
        } else if self
            .model
            .controller
            .pending_local_room_switch_target
            .as_deref()
            == Some(room_name.as_str())
        {
            self.model.controller.pending_local_room_switch_target = None;
        }
        self.model.room.name = Some(room_name);
    }

    pub(super) fn is_controlled_room_name(room_name: &str) -> bool {
        if !room_name.starts_with('+') {
            return false;
        }
        let Some((_, hash)) = room_name.rsplit_once(':') else {
            return false;
        };
        hash.len() == 12 && hash.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    pub(super) fn normalize_runtime_controlled_room_input_legacy_compatible(
        room: String,
    ) -> (String, Option<String>) {
        let parts: Vec<_> = room.split(':').collect();
        if !room.starts_with('+') || parts.len() < 3 {
            return (room, None);
        }

        let canonical_room = format!("{}:{}", parts[0], parts[1]);
        let normalized_password = Self::normalize_control_password_legacy_compatible(parts[2]);
        (
            canonical_room,
            (!normalized_password.is_empty()).then_some(normalized_password),
        )
    }

    pub(super) fn normalize_control_password_legacy_compatible(password: &str) -> String {
        password
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_uppercase()
    }

    pub(super) fn local_user_ready(&self) -> bool {
        self.model
            .connection
            .username
            .as_deref()
            .and_then(|username| self.model.room.users.get(username))
            .is_some_and(|user_view| user_view.ready == Some(true))
    }

    pub(super) fn user_ready_with_file(user_view: &ClientUserView) -> Option<bool> {
        user_view.file.as_ref()?;
        user_view.ready
    }

    pub(super) fn all_users_in_current_room_ready(&self) -> bool {
        if !self.local_user_ready() {
            return false;
        }
        let require_same_filenames = self.model.readiness.config.autoplay_require_same_filenames;
        self.all_other_users_in_current_room_ready()
            && (!require_same_filenames
                || self.all_users_in_current_room_match_filename_or_strong_media())
    }

    pub(super) fn all_other_users_in_current_room_ready(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };

        self.model.room.users.iter().all(|(username, user_view)| {
            if username == local_username {
                return true;
            }
            if user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            Self::user_ready_with_file(user_view) != Some(false)
        })
    }

    pub(super) fn users_in_current_room_count_for_threshold(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        // Legacy usersInRoomCount adds the current user and only counts other room users
        // where isReadyWithFile() is truthy.
        let ready_others = self
            .model
            .room
            .users
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        1 + ready_others
    }

    pub(super) fn all_users_in_current_room_match_filename_or_strong_media(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };
        let Some(local_file_name) = self.current_user_file_name() else {
            return false;
        };

        self.model.room.users.iter().all(|(username, user_view)| {
            if username == local_username || user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            if user_view
                .file
                .as_ref()
                .and_then(|file| file.name.as_deref())
                .is_some_and(|other_file_name| {
                    Self::same_filename_legacy_like(local_file_name, other_file_name)
                })
            {
                return true;
            }
            self.model.room.media_match_peer_tiers.get(username) == Some(&MediaMatchTier::Strong)
        })
    }

    pub(super) fn ready_user_count_in_current_room(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        let mut ready_count = usize::from(self.local_user_ready());
        ready_count += self
            .model
            .room
            .users
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        ready_count
    }

    pub(super) fn playlist_restore_intent_from_room_playlist(
        playlist: &RoomPlaylistView,
    ) -> Option<ReconnectPlaylistRestoreIntent> {
        if playlist.files.is_empty() {
            return None;
        }

        let index = playlist.index.filter(|index| {
            usize::try_from(*index).is_ok_and(|index| index < playlist.files.len())
        });

        Some(ReconnectPlaylistRestoreIntent {
            files: playlist.files.clone(),
            index,
        })
    }

    pub(super) fn start_autoplay_countdown(&mut self) {
        if !self.model.readiness.autoplay_timer_running {
            self.model.readiness.autoplay_time_left_seconds =
                self.model.readiness.config.autoplay_delay_seconds;
            self.model.readiness.autoplay_timer_running = true;
        }
    }

    pub(super) fn stop_autoplay_countdown(&mut self) {
        self.model.readiness.autoplay_timer_running = false;
        self.model.readiness.autoplay_time_left_seconds =
            self.model.readiness.config.autoplay_delay_seconds;
    }

    pub(super) fn resolve_room_for_playlist_update(&self, set_by: Option<&str>) -> Option<String> {
        set_by
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.model.room.name.clone())
    }

    pub(super) fn set_user_room(&mut self, username: &str, room_name: Option<String>) {
        let (previous_room, ready) = {
            let user_view = self
                .model
                .room
                .users
                .entry(username.to_owned())
                .or_default();
            let previous_room = user_view.room.clone();
            let ready = user_view.ready;
            user_view.room = room_name.clone();
            (previous_room, ready)
        };

        if previous_room != room_name
            && let Some(previous_room_name) = previous_room.as_deref()
        {
            let _ = self
                .model
                .room
                .domain
                .leave_room(username, previous_room_name);
        }
        if previous_room != room_name {
            if self.model.connection.username.as_deref() == Some(username) {
                self.model.room.media_match_peer_tiers.clear();
            } else {
                self.model.room.media_match_peer_tiers.remove(username);
            }
        }

        if let Some(new_room_name) = room_name.as_deref() {
            self.model.room.known_rooms.insert(new_room_name.to_owned());
            self.model
                .room
                .domain
                .join_room_with_ready(username, new_room_name, ready);
        }
    }

    pub(super) fn set_user_ready(&mut self, username: &str, ready: bool) {
        self.set_user_ready_state(username, Some(ready));
    }

    pub(super) fn set_user_ready_state(&mut self, username: &str, ready: Option<bool>) {
        let room_name = {
            let user_view = self
                .model
                .room
                .users
                .entry(username.to_owned())
                .or_default();
            user_view.ready = ready;
            user_view.room.clone()
        };

        if let Some(room_name) = room_name {
            self.model
                .room
                .domain
                .join_room_with_ready(username, &room_name, ready);
        }
    }

    pub(super) fn set_user_file(&mut self, username: &str, file: Option<SharedFile>) {
        let user_view = self
            .model
            .room
            .users
            .entry(username.to_owned())
            .or_default();
        let file_changed = user_view.file != file;
        user_view.file = file;
        if file_changed {
            if self.model.connection.username.as_deref() == Some(username) {
                self.model.room.media_match_peer_tiers.clear();
            } else {
                self.model.room.media_match_peer_tiers.remove(username);
            }
        }
    }

    pub(super) fn set_user_controller(&mut self, username: &str, controller: bool) {
        let user_view = self
            .model
            .room
            .users
            .entry(username.to_owned())
            .or_default();
        user_view.controller = controller;
    }

    pub(super) fn set_user_capabilities(
        &mut self,
        username: &str,
        capabilities: Option<PeerCapabilities>,
    ) {
        let user_view = self
            .model
            .room
            .users
            .entry(username.to_owned())
            .or_default();
        user_view.capabilities = capabilities;
    }

    pub(super) fn remove_user(&mut self, username: &str) {
        if let Some(user_view) = self.model.room.users.remove(username)
            && let Some(room_name) = user_view.room
        {
            let _ = self.model.room.domain.leave_room(username, &room_name);
        }
        self.model.room.media_match_peer_tiers.remove(username);
    }
}
