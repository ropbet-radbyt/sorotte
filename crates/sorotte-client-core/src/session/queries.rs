use super::*;

impl ClientSession {
    pub fn user_room(&self, username: &str) -> Option<&str> {
        self.user_views
            .get(username)
            .and_then(|user| user.room.as_deref())
    }

    pub fn room_names(&self) -> Vec<String> {
        let mut rooms = self.known_rooms.iter().cloned().collect::<Vec<_>>();
        if let Some(current_room) = self
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && !rooms.iter().any(|room| room == current_room)
        {
            rooms.push(current_room.to_owned());
            rooms.sort();
        }
        rooms
    }

    pub fn usernames_in_room(&self, room_name: &str) -> Vec<String> {
        self.user_views
            .iter()
            .filter_map(|(username, user)| {
                (!username.trim().is_empty() && user.room.as_deref() == Some(room_name))
                    .then_some(username.clone())
            })
            .collect()
    }

    pub fn user_ready(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).and_then(|user| user.ready)
    }

    pub fn user_has_file(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).map(|user| user.has_file)
    }

    pub fn user_file_name(&self, username: &str) -> Option<&str> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_name.as_deref())
    }

    pub fn user_file_size(&self, username: &str) -> Option<&Value> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_size.as_ref())
    }

    pub fn user_file_duration(&self, username: &str) -> Option<&Value> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_duration.as_ref())
    }

    pub fn user_controller(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).map(|user| user.controller)
    }

    pub fn user_features(&self, username: &str) -> Option<&Value> {
        self.user_views
            .get(username)
            .and_then(|user| user.features.as_ref())
    }

    pub fn file_differences_for_user(&self, username: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.username.as_deref()?;
        let current_user = self.user_views.get(current_username)?;
        let other_user = self.user_views.get(username)?;
        if current_user.room.is_none() || current_user.room != other_user.room {
            return None;
        }
        Self::file_difference_summary_for_users(current_user, other_user, self)
    }

    pub fn file_differences_for_room(&self, room_name: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.username.as_deref()?;
        let current_user = self.user_views.get(current_username)?;
        if current_user.room.as_deref() != Some(room_name) {
            return None;
        }

        let mut summary = FileDifferenceSummary::default();
        let mut compared_any = false;
        for (username, user_view) in &self.user_views {
            if username == current_username {
                continue;
            }
            if user_view.room.as_deref() != Some(room_name) {
                continue;
            }
            if let Some(user_summary) =
                Self::file_difference_summary_for_users(current_user, user_view, self)
            {
                compared_any = true;
                summary.filename |= user_summary.filename;
                summary.filesize |= user_summary.filesize;
                summary.fileduration |= user_summary.fileduration;
            }
        }

        if compared_any { Some(summary) } else { None }
    }

    pub fn file_differences_for_current_room(&self) -> Option<FileDifferenceSummary> {
        let room_name = self.room.as_deref()?;
        self.file_differences_for_room(room_name)
    }

    pub fn same_filename_legacy_compatible(left: &str, right: &str) -> bool {
        Self::same_filename_legacy_like(left, right)
    }

    pub fn same_filesize_legacy_compatible(left: &Value, right: &Value) -> bool {
        Self::same_filesize_legacy_like(left, right)
    }

    pub fn same_fileduration_legacy_compatible(left: f64, right: f64) -> bool {
        Self::same_fileduration_legacy_compatible_with_overrides(
            left,
            right,
            LEGACY_SHOW_DURATION_NOTIFICATION,
            LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS,
        )
    }

    pub fn same_fileduration_legacy_compatible_with_overrides(
        left: f64,
        right: f64,
        show_duration_notification: bool,
        different_duration_threshold_seconds: f64,
    ) -> bool {
        Self::same_fileduration_legacy_like(
            left,
            right,
            show_duration_notification,
            different_duration_threshold_seconds,
        )
    }

    pub fn same_fileduration_with_readiness_autoplay_config(&self, left: f64, right: f64) -> bool {
        Self::same_fileduration_legacy_compatible_with_overrides(
            left,
            right,
            self.readiness_autoplay_config.show_duration_notification,
            self.readiness_autoplay_config
                .different_duration_threshold_seconds,
        )
    }

    pub fn sanitize_outbound_file_payload_legacy_compatible(
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Option<Value> {
        let Value::Object(file_map) = file_payload else {
            return None;
        };

        let mut sanitized = file_map.clone();
        sanitized.remove("path");
        let has_file_metadata = file_map.contains_key("name")
            || file_map.contains_key("duration")
            || file_map.contains_key("size");

        if let Some(name_value) = file_map.get("name") {
            let sanitized_name =
                Self::filename_with_privacy_mode_legacy_like(name_value, filename_privacy_mode);
            if let Some(sanitized_name) = sanitized_name {
                sanitized.insert("name".to_owned(), Value::String(sanitized_name));
            }
        }

        if has_file_metadata {
            let duration_value = file_map
                .get("duration")
                .and_then(Value::as_f64)
                .map(Value::from)
                .unwrap_or_else(|| Value::from(0.0));
            sanitized.insert("duration".to_owned(), duration_value);

            let size_value = file_map
                .get("size")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| Value::from(0));
            let sanitized_size =
                Self::filesize_with_privacy_mode_legacy_like(&size_value, filesize_privacy_mode);
            if let Some(sanitized_size) = sanitized_size {
                sanitized.insert("size".to_owned(), sanitized_size);
            }
        }

        Some(Value::Object(sanitized))
    }

    pub fn runtime_actions_for_local_file_publish_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Vec<ClientRuntimeAction> {
        let Some(sanitized_payload) = Self::sanitize_outbound_file_payload_legacy_compatible(
            file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        ) else {
            return Vec::new();
        };

        if let Some(username) = self.username.clone() {
            let (has_file, file_name, file_size, file_duration) =
                Self::list_payload_file_info(Some(&sanitized_payload));
            self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
        }

        let mut actions = vec![ClientRuntimeAction::SetFile {
            file_payload: sanitized_payload,
        }];
        actions.push(ClientRuntimeAction::RequestUserList);
        actions
    }

    pub fn runtime_actions_for_local_media_opened_not_ready(&mut self) -> Vec<ClientRuntimeAction> {
        if self.server_readiness_supported != Some(true) {
            return Vec::new();
        }
        let Some(username) = self.username.clone() else {
            return Vec::new();
        };

        self.set_user_ready_state(&username, Some(false));
        vec![ClientRuntimeAction::SetReady {
            ready: false,
            manually_initiated: false,
        }]
    }

    pub fn room_playlist(&self, room_name: &str) -> Option<&RoomPlaylistView> {
        self.room_playlists.get(room_name)
    }

    pub fn current_room_playlist(&self) -> Option<&RoomPlaylistView> {
        self.room
            .as_deref()
            .and_then(|room_name| self.room_playlists.get(room_name))
    }

    pub(super) fn playlist_target_for_room_index(
        &self,
        room_name: &str,
        index: i64,
    ) -> Option<&str> {
        let index = usize::try_from(index).ok()?;
        self.room_playlists
            .get(room_name)
            .and_then(|playlist| playlist.files.get(index))
            .map(String::as_str)
    }

    pub(crate) fn apply_local_playlist_runtime_actions_legacy_compatible(
        &mut self,
        actions: &[ClientRuntimeAction],
    ) {
        let Some(room_name) = self.room.clone() else {
            return;
        };
        let Some(local_username) = self.username.clone() else {
            return;
        };

        let mut playlist = self
            .room_playlists
            .get(&room_name)
            .cloned()
            .unwrap_or_default();
        let mut playlist_changed = false;
        for action in actions {
            match action {
                ClientRuntimeAction::SetPlaylist { files } => {
                    playlist.files = files.clone();
                    if playlist.files.is_empty() {
                        playlist.index = None;
                    }
                    playlist.set_by = Some(local_username.clone());
                    playlist_changed = true;
                }
                ClientRuntimeAction::SetPlaylistIndex { index } => {
                    let Ok(index_usize) = usize::try_from(*index) else {
                        continue;
                    };
                    if index_usize >= playlist.files.len() {
                        continue;
                    }
                    playlist.index = Some(*index);
                    playlist.set_by = Some(local_username.clone());
                    playlist_changed = true;
                }
                _ => {}
            }
        }
        if playlist_changed {
            self.room_playlists.insert(room_name, playlist);
        }
    }

    pub fn room_playstate(&self, room_name: &str) -> Option<&RoomPlaystateView> {
        self.room_playstates.get(room_name)
    }

    pub fn current_room_playstate(&self) -> Option<&RoomPlaystateView> {
        self.room
            .as_deref()
            .and_then(|room_name| self.room_playstates.get(room_name))
    }

    pub fn current_room_playstate_at(&self, now_seconds: f64) -> Option<RoomPlaystateView> {
        let room_name = self.room.as_deref()?;
        let mut playstate = self.room_playstates.get(room_name)?.clone();
        let updated_at_seconds = self
            .room_playstate_updated_at_seconds
            .get(room_name)
            .copied();
        if playstate.paused == Some(false)
            && let (Some(position), Some(updated_at_seconds)) =
                (playstate.position, updated_at_seconds)
        {
            let elapsed_seconds = now_seconds - updated_at_seconds;
            if elapsed_seconds.is_finite() && elapsed_seconds > 0.0 {
                playstate.position = Some(position + elapsed_seconds);
            }
        }
        Some(playstate)
    }

    pub fn current_room_playstate_has_remote_authority(&self) -> bool {
        self.current_room_playstate()
            .is_some_and(|playstate| self.room_playstate_has_remote_authority(playstate))
    }

    pub fn client_ignoring_on_the_fly(&self) -> u32 {
        self.client_ignoring_on_the_fly
    }

    pub fn server_ignoring_on_the_fly(&self) -> u32 {
        self.server_ignoring_on_the_fly
    }

    pub fn desync_config(&self) -> &DesyncCorrectionConfig {
        &self.desync_config
    }

    pub fn desync_config_mut(&mut self) -> &mut DesyncCorrectionConfig {
        &mut self.desync_config
    }

    pub fn reconnect_policy(&self) -> &ReconnectPolicyConfig {
        &self.reconnect_policy
    }

    pub fn reconnect_policy_mut(&mut self) -> &mut ReconnectPolicyConfig {
        &mut self.reconnect_policy
    }

    pub fn behavior_config(&self) -> &SessionBehaviorConfig {
        &self.behavior_config
    }

    pub fn behavior_config_mut(&mut self) -> &mut SessionBehaviorConfig {
        &mut self.behavior_config
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        &self.reconnect_state_restore_correction_metrics
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        ReconnectStateRestoreCorrectionStateSnapshot {
            validation_pending: self.reconnect_state_restore_validation_pending,
            retry_attempts: self.reconnect_state_restore_validation_retry_attempts,
            retry_cooldown_ticks: self.reconnect_state_restore_validation_retry_cooldown_ticks,
            mismatch_notified_in_cycle: self.reconnect_state_restore_validation_mismatch_notified,
            mismatch_seen_in_cycle: self.reconnect_state_restore_validation_mismatch_seen_in_cycle,
            effective_policy_mode: self.reconnect_state_restore_correction_policy_mode(),
            position_tolerance_seconds: self
                .reconnect_state_restore_position_tolerance_seconds_effective(),
            effective_retry_max_attempts: self
                .reconnect_state_restore_correction_effective_retry_max_attempts(),
            consecutive_mismatch_cycles: self
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            consecutive_retry_exhaustions: self
                .reconnect_state_restore_correction_consecutive_retry_exhaustions,
            recovery_cooldown_reconnect_cycles_remaining: self
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            correction_suppressed_for_recovery_cycle: self
                .reconnect_state_restore_correction_recovery_suppressed_this_cycle,
            correction_reenabled_for_recovery_cycle: self
                .reconnect_state_restore_correction_recovery_reenabled_this_cycle,
        }
    }

    pub fn readiness_autoplay_config(&self) -> &ReadinessAutoplayConfig {
        &self.readiness_autoplay_config
    }

    pub fn readiness_autoplay_config_mut(&mut self) -> &mut ReadinessAutoplayConfig {
        &mut self.readiness_autoplay_config
    }

    pub fn chat_config(&self) -> &ChatConfig {
        &self.chat_config
    }

    pub fn chat_config_mut(&mut self) -> &mut ChatConfig {
        &mut self.chat_config
    }

    pub fn last_paused_on_leave_at_seconds(&self) -> Option<f64> {
        self.last_paused_on_leave_at_seconds
    }

    pub fn server_readiness_supported(&self) -> Option<bool> {
        self.server_readiness_supported
    }

    pub fn server_set_others_readiness_supported(&self) -> Option<bool> {
        self.server_set_others_readiness_supported
    }

    pub fn server_managed_rooms_supported(&self) -> Option<bool> {
        self.server_managed_rooms_supported
    }

    pub fn server_shared_playlists_supported(&self) -> Option<bool> {
        self.server_shared_playlists_supported
    }

    pub fn server_chat_supported(&self) -> Option<bool> {
        self.server_chat_supported
    }

    pub fn server_persistent_rooms_supported(&self) -> Option<bool> {
        self.server_persistent_rooms_supported
    }

    pub fn server_max_username_length(&self) -> Option<usize> {
        self.server_max_username_length
    }

    pub fn server_max_room_name_length(&self) -> Option<usize> {
        self.server_max_room_name_length
    }

    pub fn server_max_filename_length(&self) -> Option<usize> {
        self.server_max_filename_length
    }

    pub fn clear_server_feature_support_state(&mut self) {
        self.server_readiness_supported = None;
        self.server_set_others_readiness_supported = None;
        self.server_managed_rooms_supported = None;
        self.server_shared_playlists_supported = None;
        self.server_chat_supported = None;
        self.server_persistent_rooms_supported = None;
        self.server_max_username_length = None;
        self.server_max_room_name_length = None;
        self.server_max_filename_length = None;
    }

    pub fn local_can_control(&self) -> Option<bool> {
        let username = self.username.as_deref()?;
        let room_name = self.room.as_deref()?;
        if !Self::is_controlled_room_name(room_name) {
            return Some(true);
        }
        Some(self.user_controller(username).unwrap_or(false))
    }

    pub fn noncontroller_event_hide_from_osd_legacy_compatible(&self, username: &str) -> bool {
        !self.behavior_config.show_noncontroller_osd && self.user_controller(username) != Some(true)
    }

    pub(super) fn show_user_change_event_on_osd_legacy_compatible(
        &self,
        current_room: Option<&str>,
        previous_room: Option<&str>,
        username: &str,
    ) -> bool {
        let local_room = self.room.as_deref();
        let room_matches_local = local_room.is_some_and(|local_room| {
            current_room == Some(local_room) || previous_room == Some(local_room)
        });

        let mut show_on_osd = if room_matches_local {
            self.behavior_config.show_osd_warnings
        } else {
            self.behavior_config.show_different_room_osd
        };

        if !self.behavior_config.show_noncontroller_osd
            && self.user_controller(username) != Some(true)
        {
            show_on_osd = false;
        }

        show_on_osd
    }

    pub(super) fn queue_user_left_notification_if_relevant(
        &mut self,
        username: &str,
        previous_user_view: Option<ClientUserView>,
    ) {
        let Some(previous_user_view) = previous_user_view else {
            return;
        };

        let previous_room = previous_user_view.room.as_deref();
        let local_room = self.room.as_deref();
        let show_on_osd = if local_room == previous_room {
            self.behavior_config.show_same_room_osd
        } else {
            self.behavior_config.show_different_room_osd
        };

        self.pending_user_change_notifications
            .push(UserChangeNotification::Left {
                username: username.to_owned(),
                hide_from_osd: !show_on_osd,
            });
    }

    pub(super) fn queue_user_change_notification_if_relevant(
        &mut self,
        username: &str,
        previous_user_view: Option<ClientUserView>,
    ) {
        let Some(current_user_view) = self.user_views.get(username).cloned() else {
            return;
        };
        let Some(room_name) = current_user_view.room.clone() else {
            return;
        };

        let room_changed = previous_user_view
            .as_ref()
            .and_then(|view| view.room.as_deref())
            != Some(room_name.as_str());
        let file_changed = match previous_user_view.as_ref() {
            Some(previous_user_view) => {
                previous_user_view.has_file != current_user_view.has_file
                    || previous_user_view.file_name != current_user_view.file_name
                    || previous_user_view.file_size != current_user_view.file_size
                    || previous_user_view.file_duration != current_user_view.file_duration
            }
            None => current_user_view.has_file,
        };

        if !room_changed && !file_changed {
            return;
        }

        let previous_room = previous_user_view
            .as_ref()
            .and_then(|view| view.room.as_deref());
        let show_on_osd = self.show_user_change_event_on_osd_legacy_compatible(
            Some(room_name.as_str()),
            previous_room,
            username,
        );
        let hide_from_osd = !show_on_osd;
        if current_user_view.has_file {
            let include_room_addendum = self.room.as_deref() != Some(room_name.as_str());
            self.pending_user_change_notifications
                .push(UserChangeNotification::Playing {
                    username: username.to_owned(),
                    room: room_name,
                    file_name: current_user_view.file_name,
                    file_duration: current_user_view.file_duration,
                    include_room_addendum,
                    hide_from_osd,
                });
        } else if room_changed {
            self.pending_user_change_notifications
                .push(UserChangeNotification::Joined {
                    username: username.to_owned(),
                    room: room_name,
                    hide_from_osd,
                });
        }
    }

    pub fn remember_control_password_for_room(&mut self, room_name: &str, password: &str) {
        if !Self::is_controlled_room_name(room_name) {
            return;
        }

        let normalized_password = Self::normalize_control_password_legacy_compatible(password);
        if normalized_password.is_empty() {
            return;
        }

        self.controlled_room_passwords
            .insert(room_name.to_owned(), normalized_password);
    }

    pub fn autoplay_enabled(&self) -> bool {
        self.autoplay_enabled
    }

    pub fn local_paused(&self) -> Option<bool> {
        self.local_paused
    }

    pub fn local_paused_for_cache(&self) -> Option<bool> {
        self.local_paused_for_cache
    }

    pub fn local_cache_buffering_percent(&self) -> Option<f64> {
        self.local_cache_buffering_percent
    }

    pub fn local_position_seconds(&self) -> Option<f64> {
        self.local_position
    }

    pub fn last_seek_position_before_manual_seek(&self) -> Option<f64> {
        self.last_seek_position_before_manual_seek
    }

    pub fn set_autoplay_enabled(&mut self, enabled: bool) {
        self.autoplay_enabled = enabled;
        if !enabled {
            self.stop_autoplay_countdown();
        }
    }

    pub fn set_strong_same_media_match_satisfies_filename_gate(&mut self, satisfied: bool) {
        self.readiness_autoplay_config
            .strong_same_media_match_satisfies_filename_gate = satisfied;
    }

    pub fn autoplay_timer_is_running(&self) -> bool {
        self.autoplay_timer_running
    }

    pub fn autoplay_time_left_seconds(&self) -> f64 {
        self.autoplay_time_left_seconds
    }

    pub fn is_playing_music(&self) -> bool {
        self.current_user_file_name()
            .is_some_and(Self::is_music_file_name)
    }
}
