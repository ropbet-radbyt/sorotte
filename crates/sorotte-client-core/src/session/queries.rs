use super::*;

impl ClientSession {
    pub fn drain_compatibility_fallbacks(&mut self) -> Vec<ClientCompatibilityFallback> {
        std::mem::take(&mut self.pending_compatibility_fallbacks)
    }

    pub fn username(&self) -> Option<&str> {
        self.model.connection.username.as_deref()
    }

    pub fn room(&self) -> Option<&str> {
        self.model.room.name.as_deref()
    }

    pub fn model(&self) -> &ClientModel {
        &self.model
    }

    pub fn user_room(&self, username: &str) -> Option<&str> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.room.as_deref())
    }

    pub fn room_names(&self) -> Vec<String> {
        let mut rooms = self
            .model
            .room
            .known_rooms
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(current_room) = self
            .model
            .room
            .name
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
        self.model
            .room
            .users
            .iter()
            .filter_map(|(username, user)| {
                (!username.trim().is_empty() && user.room.as_deref() == Some(room_name))
                    .then_some(username.clone())
            })
            .collect()
    }

    pub fn user_ready(&self, username: &str) -> Option<bool> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.ready)
    }

    pub fn user_has_file(&self, username: &str) -> Option<bool> {
        self.model
            .room
            .users
            .get(username)
            .map(|user| user.file.is_some())
    }

    pub fn user_file_name(&self, username: &str) -> Option<&str> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.file.as_ref())
            .and_then(|file| file.name.as_deref())
    }

    pub fn user_file_size(&self, username: &str) -> Option<&FileSize> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.file.as_ref())
            .and_then(|file| file.size.as_ref())
    }

    pub fn user_file_duration(&self, username: &str) -> Option<f64> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.file.as_ref())
            .and_then(|file| file.duration)
            .map(FileDuration::as_seconds)
    }

    pub fn user_file_duration_wire(&self, username: &str) -> Option<FileDuration> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.file.as_ref())
            .and_then(|file| file.duration)
    }

    pub fn user_controller(&self, username: &str) -> Option<bool> {
        self.model
            .room
            .users
            .get(username)
            .map(|user| user.controller)
    }

    pub fn user_capabilities(&self, username: &str) -> Option<&PeerCapabilities> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user| user.capabilities.as_ref())
    }

    pub fn file_differences_for_user(&self, username: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.model.connection.username.as_deref()?;
        let current_user = self.model.room.users.get(current_username)?;
        let other_user = self.model.room.users.get(username)?;
        if current_user.room.is_none() || current_user.room != other_user.room {
            return None;
        }
        Self::file_difference_summary_for_users(current_user, other_user, self)
    }

    pub fn file_differences_for_room(&self, room_name: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.model.connection.username.as_deref()?;
        let current_user = self.model.room.users.get(current_username)?;
        if current_user.room.as_deref() != Some(room_name) {
            return None;
        }

        let mut summary = FileDifferenceSummary::default();
        let mut compared_any = false;
        for (username, user_view) in &self.model.room.users {
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
        let room_name = self.model.room.name.as_deref()?;
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
            self.model.readiness.config.show_duration_notification,
            self.model
                .readiness
                .config
                .different_duration_threshold_seconds,
        )
    }

    pub fn sanitize_outbound_file_payload_legacy_compatible(
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Option<FilePayload> {
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

        let name = sanitized
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned));
        let duration = sanitized
            .remove("duration")
            .and_then(|value| value.as_f64());
        let size = sanitized.remove("size");
        let path = sanitized
            .remove("path")
            .and_then(|value| value.as_str().map(str::to_owned));

        Some(FilePayload {
            name,
            duration,
            size,
            path,
            extra: sanitized.into_iter().collect(),
        })
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

        if let Some(username) = self.model.connection.username.clone() {
            let file = Self::shared_file_from_file_payload(&sanitized_payload);
            self.set_user_file(&username, file);
        }

        let mut actions = vec![ClientRuntimeAction::SetFile {
            file: sanitized_payload,
        }];
        actions.push(ClientRuntimeAction::RequestUserList);
        actions
    }

    pub fn runtime_actions_for_local_media_opened_not_ready(&mut self) -> Vec<ClientRuntimeAction> {
        if !self.server_readiness_supported() {
            return Vec::new();
        }
        let Some(username) = self.model.connection.username.clone() else {
            return Vec::new();
        };

        self.set_user_ready_state(&username, Some(false));
        vec![ClientRuntimeAction::SetReady {
            ready: false,
            manually_initiated: false,
        }]
    }

    pub fn room_playlist(&self, room_name: &str) -> Option<&RoomPlaylistView> {
        self.model.playlist.rooms.get(room_name)
    }

    pub fn current_room_playlist(&self) -> Option<&RoomPlaylistView> {
        self.model
            .room
            .name
            .as_deref()
            .and_then(|room_name| self.model.playlist.rooms.get(room_name))
    }

    pub fn current_room_playlist_remote_revision(&self) -> u64 {
        self.model
            .room
            .name
            .as_ref()
            .and_then(|room_name| self.model.playlist.remote_revisions.get(room_name))
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn playlist_target_for_room_index(
        &self,
        room_name: &str,
        index: i64,
    ) -> Option<&str> {
        let index = usize::try_from(index).ok()?;
        self.model
            .playlist
            .rooms
            .get(room_name)
            .and_then(|playlist| playlist.files.get(index))
            .map(String::as_str)
    }

    pub(crate) fn apply_local_playlist_runtime_actions_legacy_compatible(
        &mut self,
        actions: &[ClientRuntimeAction],
    ) {
        let Some(room_name) = self.model.room.name.clone() else {
            return;
        };
        let Some(local_username) = self.model.connection.username.clone() else {
            return;
        };

        let mut playlist = self
            .model
            .playlist
            .rooms
            .get(&room_name)
            .cloned()
            .unwrap_or_default();
        let mut playlist_changed = false;
        for action in actions {
            match action {
                ClientRuntimeAction::SetPlaylist { files } => {
                    playlist.revision = playlist.revision.wrapping_add(1);
                    playlist.files = files.clone();
                    if playlist.files.is_empty() {
                        playlist.index = None;
                    }
                    playlist.set_by = Some(local_username.clone());
                    self.model
                        .playlist
                        .pending_local_change_echoes
                        .entry(room_name.clone())
                        .or_default()
                        .record(playlist.revision, files);
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
                    self.model
                        .playlist
                        .pending_local_index_echoes
                        .entry(room_name.clone())
                        .or_default()
                        .record(playlist.revision, *index);
                    playlist_changed = true;
                }
                _ => {}
            }
        }
        if playlist_changed {
            self.model.playlist.rooms.insert(room_name, playlist);
        }
    }

    pub fn room_playstate(&self, room_name: &str) -> Option<&RoomPlaystateView> {
        self.model.room.playstates.get(room_name)
    }

    pub fn current_room_playstate(&self) -> Option<&RoomPlaystateView> {
        self.model
            .room
            .name
            .as_deref()
            .and_then(|room_name| self.model.room.playstates.get(room_name))
    }

    pub fn current_room_playstate_at(&self, now_seconds: f64) -> Option<RoomPlaystateView> {
        let room_name = self.model.room.name.as_deref()?;
        let mut playstate = self.model.room.playstates.get(room_name)?.clone();
        let updated_at_seconds = self
            .model
            .room
            .playstate_updated_at_seconds
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

    pub fn current_room_playstate_authority(&self) -> Option<RoomPlaystateAuthority> {
        let playstate = self.current_room_playstate()?;
        if let (Some(prepare), Some(status)) = (
            self.playback_barrier_prepare(),
            self.playback_barrier_status(),
        ) && prepare.media_generation == status.media_generation
            && matches!(
                status.phase,
                sorotte_protocol::PlaybackBarrierPhase::Preparing
                    | sorotte_protocol::PlaybackBarrierPhase::Committed
                    | sorotte_protocol::PlaybackBarrierPhase::AwaitingDecision
            )
        {
            return Some(RoomPlaystateAuthority::ServerBarrier {
                media_generation: status.media_generation,
                state_revision: status.state_revision,
            });
        }
        if let Some(status) = self.playback_barrier_buffering_status()
            && matches!(
                status.phase,
                sorotte_protocol::RoomBufferingPhase::Paused
                    | sorotte_protocol::RoomBufferingPhase::DebouncingResume
            )
        {
            return Some(RoomPlaystateAuthority::ServerBufferingPolicy {
                media_generation: status.config.media_generation,
            });
        }
        match playstate.set_by.as_deref() {
            Some(set_by) if self.model.connection.username.as_deref() == Some(set_by) => {
                Some(RoomPlaystateAuthority::LegacyLocalEcho)
            }
            Some(_) => Some(RoomPlaystateAuthority::LegacyRemoteUser),
            None if self.current_room_has_other_users() => {
                Some(RoomPlaystateAuthority::LegacyRemoteUser)
            }
            None => None,
        }
    }

    pub fn client_ignoring_on_the_fly(&self) -> u32 {
        self.model.playback.client_ignoring_on_the_fly
    }

    pub fn server_ignoring_on_the_fly(&self) -> u32 {
        self.model.playback.server_ignoring_on_the_fly
    }

    pub fn desync_config(&self) -> &DesyncCorrectionConfig {
        &self.model.playback.desync_config
    }

    pub fn desync_config_mut(&mut self) -> &mut DesyncCorrectionConfig {
        &mut self.model.playback.desync_config
    }

    pub(crate) fn set_desync_config(&mut self, config: DesyncCorrectionConfig) {
        self.model.playback.desync_config = config;
    }

    pub fn reconnect_policy(&self) -> &ReconnectPolicyConfig {
        &self.model.reconnect.policy
    }

    pub fn reconnect_policy_mut(&mut self) -> &mut ReconnectPolicyConfig {
        &mut self.model.reconnect.policy
    }

    pub(crate) fn set_reconnect_policy(&mut self, policy: ReconnectPolicyConfig) {
        self.model.reconnect.policy = policy;
    }

    pub fn behavior_config(&self) -> &SessionBehaviorConfig {
        &self.behavior_config
    }

    pub fn behavior_config_mut(&mut self) -> &mut SessionBehaviorConfig {
        &mut self.behavior_config
    }

    pub(crate) fn set_behavior_config(&mut self, config: SessionBehaviorConfig) {
        self.behavior_config = config;
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        &self.model.reconnect.state_restore_correction_metrics
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        ReconnectStateRestoreCorrectionStateSnapshot {
            validation_pending: self.model.reconnect.state_restore_validation_pending,
            retry_attempts: self.model.reconnect.state_restore_validation_retry_attempts,
            retry_cooldown_ticks: self
                .model
                .reconnect
                .state_restore_validation_retry_cooldown_ticks,
            mismatch_notified_in_cycle: self
                .model
                .reconnect
                .state_restore_validation_mismatch_notified,
            mismatch_seen_in_cycle: self
                .model
                .reconnect
                .state_restore_validation_mismatch_seen_in_cycle,
            effective_policy_mode: self.reconnect_state_restore_correction_policy_mode(),
            position_tolerance_seconds: self
                .reconnect_state_restore_position_tolerance_seconds_effective(),
            effective_retry_max_attempts: self
                .reconnect_state_restore_correction_effective_retry_max_attempts(),
            consecutive_mismatch_cycles: self
                .model
                .reconnect
                .state_restore_correction_consecutive_mismatch_cycles,
            consecutive_retry_exhaustions: self
                .model
                .reconnect
                .state_restore_correction_consecutive_retry_exhaustions,
            recovery_cooldown_reconnect_cycles_remaining: self
                .model
                .reconnect
                .state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            correction_suppressed_for_recovery_cycle: self
                .model
                .reconnect
                .state_restore_correction_recovery_suppressed_this_cycle,
            correction_reenabled_for_recovery_cycle: self
                .model
                .reconnect
                .state_restore_correction_recovery_reenabled_this_cycle,
        }
    }

    pub fn readiness_autoplay_config(&self) -> &ReadinessAutoplayConfig {
        &self.model.readiness.config
    }

    pub fn readiness_autoplay_config_mut(&mut self) -> &mut ReadinessAutoplayConfig {
        &mut self.model.readiness.config
    }

    pub(crate) fn set_readiness_autoplay_config(&mut self, config: ReadinessAutoplayConfig) {
        self.model.readiness.config = config;
    }

    pub fn chat_config(&self) -> &ChatConfig {
        &self.chat_config
    }

    pub fn chat_config_mut(&mut self) -> &mut ChatConfig {
        &mut self.chat_config
    }

    pub(crate) fn set_chat_config(&mut self, config: ChatConfig) {
        self.chat_config = config;
    }

    pub fn last_paused_on_leave_at_seconds(&self) -> Option<f64> {
        self.model.playback.last_paused_on_leave_at_seconds
    }

    pub fn connection_phase(&self) -> &ConnectionPhase {
        &self.model.connection.phase
    }

    pub fn is_active(&self) -> bool {
        matches!(self.connection_phase(), ConnectionPhase::Active(_))
    }

    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        match self.connection_phase() {
            ConnectionPhase::Active(capabilities) => Some(capabilities),
            _ => None,
        }
    }

    pub fn mark_connecting(&mut self) {
        self.model.connection.phase = ConnectionPhase::Connecting;
    }

    pub fn mark_awaiting_hello(&mut self) {
        self.model.connection.phase = ConnectionPhase::AwaitingHello;
    }

    pub fn mark_reconnecting(&mut self, attempt: u32) {
        self.model.connection.phase = ConnectionPhase::Reconnecting { attempt };
    }

    pub fn mark_closing(&mut self) {
        self.model.connection.phase = ConnectionPhase::Closing;
    }

    pub fn mark_disconnected(&mut self) {
        self.model.connection.phase = ConnectionPhase::Disconnected;
    }

    pub fn server_readiness_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.readiness)
    }

    pub fn server_set_others_readiness_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.remote_readiness)
    }

    pub fn server_managed_rooms_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.managed_rooms)
    }

    pub fn server_shared_playlists_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.shared_playlists)
    }

    pub fn server_media_match_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.media_match)
    }

    pub fn server_chat_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.chat)
    }

    pub fn server_plex_playlist_uris_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.plex_playlist_uris)
    }

    pub fn server_persistent_rooms_supported(&self) -> bool {
        self.server_capabilities()
            .is_some_and(|capabilities| capabilities.persistent_rooms)
    }

    pub fn server_max_username_length(&self) -> Option<usize> {
        self.server_capabilities()
            .map(|capabilities| capabilities.max_username_length)
    }

    pub fn server_max_room_name_length(&self) -> Option<usize> {
        self.server_capabilities()
            .map(|capabilities| capabilities.max_room_name_length)
    }

    pub fn server_max_filename_length(&self) -> Option<usize> {
        self.server_capabilities()
            .map(|capabilities| capabilities.max_filename_length)
    }

    pub fn local_can_control(&self) -> Option<bool> {
        let username = self.model.connection.username.as_deref()?;
        let room_name = self.model.room.name.as_deref()?;
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
        let local_room = self.model.room.name.as_deref();
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
        let local_room = self.model.room.name.as_deref();
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
        let Some(current_user_view) = self.model.room.users.get(username).cloned() else {
            return;
        };
        let Some(room_name) = current_user_view.room.clone() else {
            return;
        };

        let room_changed = previous_user_view
            .as_ref()
            .and_then(|view| view.room.as_deref())
            != Some(room_name.as_str());
        let file_changed = previous_user_view
            .as_ref()
            .map_or(current_user_view.file.is_some(), |previous_user_view| {
                previous_user_view.file != current_user_view.file
            });

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
        if let Some(file) = current_user_view.file {
            let include_room_addendum = self.model.room.name.as_deref() != Some(room_name.as_str());
            self.pending_user_change_notifications
                .push(UserChangeNotification::Playing {
                    username: username.to_owned(),
                    room: room_name,
                    file_name: file.name,
                    file_duration: file.duration.map(FileDuration::as_seconds),
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

    pub fn remember_control_password_for_room(
        &mut self,
        room_name: &str,
        password: impl Into<SecretValue>,
    ) {
        let password = password.into();
        let normalized_password =
            Self::normalize_control_password_legacy_compatible(password.expose_secret());
        self.remember_normalized_control_password_for_room(
            room_name,
            SecretValue::new(normalized_password),
        );
    }

    pub(super) fn remember_normalized_control_password_for_room(
        &mut self,
        room_name: &str,
        password: SecretValue,
    ) {
        if !Self::is_controlled_room_name(room_name) || password.is_empty() {
            return;
        }

        self.model
            .controller
            .room_passwords
            .insert(room_name.to_owned(), password);
    }

    pub fn autoplay_enabled(&self) -> bool {
        self.model.readiness.autoplay_enabled
    }

    pub fn local_paused(&self) -> Option<bool> {
        self.model.playback.local_paused
    }

    pub fn local_playback_rate(&self) -> Option<f64> {
        self.model.playback.local_playback_rate
    }

    pub fn local_pause_change_health(&self) -> LocalPauseChangeHealth {
        self.model.playback.local_pause_change_health
    }

    pub fn local_paused_for_cache(&self) -> Option<bool> {
        self.model.playback.local_paused_for_cache
    }

    pub fn local_cache_buffering_percent(&self) -> Option<f64> {
        self.model.playback.local_cache_buffering_percent
    }

    pub fn local_position_seconds(&self) -> Option<f64> {
        self.model.playback.local_position
    }

    pub fn last_seek_position_before_manual_seek(&self) -> Option<f64> {
        self.model.playlist.last_seek_position_before_manual_seek
    }

    pub fn set_autoplay_enabled(&mut self, enabled: bool) {
        self.model.readiness.autoplay_enabled = enabled;
        if !enabled {
            self.stop_autoplay_countdown();
        }
    }

    pub fn set_media_match_peer_tiers(&mut self, tiers: BTreeMap<String, MediaMatchTier>) {
        self.model.room.media_match_peer_tiers = tiers;
    }

    pub fn media_match_peer_tiers(&self) -> &BTreeMap<String, MediaMatchTier> {
        &self.model.room.media_match_peer_tiers
    }

    pub fn user_media_match_signature(&self, username: &str) -> Option<&MediaMatchWireSignature> {
        self.model
            .room
            .users
            .get(username)
            .and_then(|user_view| user_view.file.as_ref())
            .and_then(|file| file.media_match.as_ref())
    }

    pub fn current_room_media_match_signatures(&self) -> Vec<(String, MediaMatchWireSignature)> {
        self.current_room_media_match_peer_file_states()
            .into_iter()
            .filter_map(|state| {
                state
                    .media_match_signature
                    .map(|signature| (state.username, signature))
            })
            .collect()
    }

    pub fn current_room_media_match_peer_file_states(&self) -> Vec<ClientMediaMatchPeerFileState> {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return Vec::new();
        };
        self.model
            .room
            .users
            .iter()
            .filter_map(|(username, user_view)| {
                if username == local_username || user_view.room.as_deref() != Some(local_room) {
                    return None;
                }
                Some(ClientMediaMatchPeerFileState {
                    username: username.clone(),
                    has_file: user_view.file.is_some(),
                    file_name: user_view.file.as_ref().and_then(|file| file.name.clone()),
                    file_size: user_view.file.as_ref().and_then(|file| file.size.clone()),
                    file_duration: user_view
                        .file
                        .as_ref()
                        .and_then(|file| file.duration)
                        .map(FileDuration::as_seconds),
                    media_match_signature: user_view
                        .file
                        .as_ref()
                        .and_then(|file| file.media_match.clone()),
                })
            })
            .collect()
    }

    pub fn autoplay_timer_is_running(&self) -> bool {
        self.model.readiness.autoplay_timer_running
    }

    pub fn autoplay_time_left_seconds(&self) -> f64 {
        self.model.readiness.autoplay_time_left_seconds
    }

    pub fn is_playing_music(&self) -> bool {
        self.current_user_file_name()
            .is_some_and(Self::is_music_file_name)
    }
}
