use super::*;

impl ClientSession {
    pub(super) fn loop_single_files_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_single_files || self.is_playing_music()
    }

    pub(super) fn loop_at_end_of_playlist_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_at_end_of_playlist || self.is_playing_music()
    }

    pub(super) fn playlist_target_switch_allowed_legacy_compatible(&self, file_name: &str) -> bool {
        if !Self::is_url(file_name) {
            return true;
        }
        self.uri_is_trusted_legacy_compatible(file_name)
    }

    pub(super) fn uri_is_trusted_legacy_compatible(&self, uri: &str) -> bool {
        let Some((host, path)) = Self::parse_trustable_web_uri_host_and_path_legacy_compatible(uri)
        else {
            return false;
        };

        if !self.behavior_config.only_switch_to_trusted_domains {
            return true;
        }

        for trusted_entry in &self.behavior_config.trusted_domains {
            let trusted_entry = trusted_entry.trim();
            if trusted_entry.is_empty() {
                continue;
            }
            let (trusted_domain, required_path_prefix) =
                trusted_entry.split_once('/').unwrap_or((trusted_entry, ""));
            let trusted_domain = trusted_domain.trim().to_ascii_lowercase();
            if trusted_domain.is_empty() {
                continue;
            }
            if !Self::trusted_domain_matches_host_legacy_compatible(&host, &trusted_domain) {
                continue;
            }
            if !required_path_prefix.is_empty() {
                let path_prefix = format!("/{required_path_prefix}");
                if !path.starts_with(&path_prefix) {
                    continue;
                }
            }
            return true;
        }
        false
    }

    pub(super) fn parse_trustable_web_uri_host_and_path_legacy_compatible(
        uri: &str,
    ) -> Option<(String, String)> {
        let uri = uri.trim();
        let authority_and_path = if let Some(value) = uri.strip_prefix("http://") {
            value
        } else {
            uri.strip_prefix("https://")?
        };
        if authority_and_path.is_empty() {
            return None;
        }

        let (authority, path_tail) = authority_and_path
            .split_once('/')
            .unwrap_or((authority_and_path, ""));
        if authority.is_empty() {
            return None;
        }

        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, value)| value);
        if authority.is_empty() {
            return None;
        }

        let host = authority
            .split(':')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }

        let path_with_query = if path_tail.is_empty() {
            "/".to_owned()
        } else {
            format!("/{path_tail}")
        };
        let path = path_with_query
            .split(['?', '#'])
            .next()
            .unwrap_or("/")
            .to_owned();
        Some((host, path))
    }

    pub(super) fn trusted_domain_matches_host_legacy_compatible(
        host: &str,
        trusted_domain: &str,
    ) -> bool {
        if host == trusted_domain || host == format!("www.{trusted_domain}") {
            return true;
        }
        if !trusted_domain.contains('*') {
            return false;
        }

        let host_parts = host.split('.').collect::<Vec<_>>();
        let pattern_parts = trusted_domain.split('.').collect::<Vec<_>>();
        if host_parts.len() != pattern_parts.len() {
            return false;
        }
        host_parts
            .iter()
            .zip(pattern_parts.iter())
            .all(|(host_part, pattern_part)| {
                if *pattern_part == "*" {
                    !host_part.is_empty()
                } else {
                    host_part.eq_ignore_ascii_case(pattern_part)
                }
            })
    }

    pub(super) fn capture_playlist_undo_snapshot_legacy_compatible(
        &mut self,
        room_name: &str,
        current_files: &[String],
        new_files: &[String],
    ) {
        if current_files == new_files {
            return;
        }
        if self
            .playlist_undo_snapshots
            .get(room_name)
            .is_some_and(|snapshot| snapshot == current_files)
        {
            return;
        }
        self.playlist_undo_snapshots
            .insert(room_name.to_owned(), current_files.to_vec());
    }

    pub(super) fn local_playlist_target_index_from_changed_playlist_legacy_compatible(
        current_files: &[String],
        current_index: Option<usize>,
        new_files: &[String],
    ) -> usize {
        let Some(current_index) = current_index else {
            return 0;
        };
        if new_files.len() <= 1 {
            return 0;
        }

        let mut index = current_index;
        while index <= current_files.len() {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return valid_index;
            }
            index = index.saturating_add(1);
        }

        let mut index = current_index;
        while index > 0 {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return if valid_index < new_files.len().saturating_sub(1) {
                    valid_index.saturating_add(1)
                } else {
                    valid_index
                };
            }
            index = index.saturating_sub(1);
        }
        0
    }

    pub(super) fn next_playlist_shuffle_seed_legacy_compatible(
        &mut self,
        files: &[String],
        current_index: usize,
        shuffle_scope_remaining: bool,
    ) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(if shuffle_scope_remaining {
            &b"remaining"[..]
        } else {
            &b"entire"[..]
        });
        hasher.update((current_index as u64).to_le_bytes());
        hasher.update(self.playlist_shuffle_nonce.to_le_bytes());
        for file_name in files {
            hasher.update(file_name.as_bytes());
            hasher.update([0]);
        }
        self.playlist_shuffle_nonce = self.playlist_shuffle_nonce.wrapping_add(1);

        let digest = hasher.finalize();
        let mut seed_bytes = [0u8; 8];
        seed_bytes.copy_from_slice(&digest[..8]);
        let seed = u64::from_le_bytes(seed_bytes);
        if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        }
    }

    pub(super) fn next_shuffle_state_legacy_compatible(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state
    }

    pub(super) fn shuffle_playlist_slice_in_place_legacy_compatible(
        files: &mut [String],
        seed: u64,
    ) {
        if files.len() <= 1 {
            return;
        }

        let mut state = seed;
        for index in (1..files.len()).rev() {
            let random_value = Self::next_shuffle_state_legacy_compatible(&mut state);
            let swap_index = (random_value as usize) % (index + 1);
            files.swap(index, swap_index);
        }
    }

    pub fn recently_advanced(&self, now_seconds: f64) -> bool {
        let threshold_seconds =
            self.readiness_autoplay_config.autoplay_delay_seconds + RECENTLY_ADVANCED_GRACE_SECONDS;
        self.last_advanced_at_seconds
            .is_some_and(|last_advanced_at_seconds| {
                let elapsed = now_seconds - last_advanced_at_seconds;
                elapsed >= 0.0 && elapsed < threshold_seconds
            })
    }

    pub fn plan_reconnect_retry(&mut self, retries: u32) -> ReconnectRetryDecision {
        self.reset_sync_state_for_reconnect();

        if retries > self.reconnect_policy.max_retries {
            self.reconnect_in_progress = false;
            self.reconnect_connected_intent = false;
            return ReconnectRetryDecision {
                should_retry: false,
                delay_seconds: None,
                should_reset_state: true,
            };
        }

        let exponent = retries.min(self.reconnect_policy.max_backoff_exponent);
        let delay_seconds = self.reconnect_policy.base_delay_seconds * 2_f64.powi(exponent as i32);
        self.reconnect_in_progress = true;

        ReconnectRetryDecision {
            should_retry: true,
            delay_seconds: Some(delay_seconds),
            should_reset_state: true,
        }
    }

    pub fn runtime_actions_for_reconnect_retry(
        &mut self,
        retries: u32,
    ) -> Vec<ClientRuntimeAction> {
        let decision = self.plan_reconnect_retry(retries);
        if decision.should_retry {
            if let Some(delay_seconds) = decision.delay_seconds {
                return vec![
                    ClientRuntimeAction::NotifyReconnectTransition(
                        ReconnectTransitionNotification::Attempting {
                            retries,
                            delay_seconds,
                        },
                    ),
                    ClientRuntimeAction::ScheduleReconnect { delay_seconds },
                ];
            }
            return Vec::new();
        }
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Disconnected,
            ),
            ClientRuntimeAction::StopReconnect,
        ]
    }

    pub fn runtime_actions_for_reconnect_transition_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.reconnect_connected_intent {
            return Vec::new();
        }
        self.reconnect_connected_intent = false;
        vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Connected,
        )]
    }

    pub fn runtime_actions_for_controller_auth_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_controller_auth_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyControllerAuthTransition)
            .collect()
    }

    pub fn runtime_actions_for_controlled_room_creation_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_controlled_room_creation_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyControlledRoomCreation)
            .collect()
    }

    pub fn runtime_actions_for_chat_notifications_if_needed(&mut self) -> Vec<ClientRuntimeAction> {
        self.pending_chat_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyChat)
            .collect()
    }

    pub fn runtime_actions_for_user_change_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_user_change_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyUserChange)
            .collect()
    }

    pub fn runtime_actions_for_reconnect_state_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let mut actions = Vec::new();

        if let Some(ready) = self.reconnect_ready_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: false,
            });
        }

        if let Some(file_payload) = self.reconnect_file_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetFile { file_payload });
            if self.server_chat_supported.is_some() {
                actions.push(ClientRuntimeAction::RequestUserList);
            }
        }

        if !actions.is_empty() {
            self.reconnect_state_restore_validation_pending = true;
            self.reconnect_state_restore_validation_retry_attempts = 0;
            self.reconnect_state_restore_validation_retry_cooldown_ticks = 0;
            self.reconnect_state_restore_validation_mismatch_notified = false;
            self.reconnect_state_restore_validation_mismatch_seen_in_cycle = false;
            self.begin_reconnect_state_restore_validation_cycle();
            actions.insert(
                0,
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringState,
                ),
            );
        }

        actions
    }

    pub fn runtime_actions_for_reconnect_state_restore_validation_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.reconnect_state_restore_validation_pending {
            return Vec::new();
        }
        if self.reconnect_state_restore_validation_retry_cooldown_ticks > 0 {
            self.reconnect_state_restore_validation_retry_cooldown_ticks = self
                .reconnect_state_restore_validation_retry_cooldown_ticks
                .saturating_sub(1);
            return Vec::new();
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let Some(room_playstate) = self.current_room_playstate_at(now_seconds) else {
            return Vec::new();
        };
        let (Some(room_paused), Some(room_position)) =
            (room_playstate.paused, room_playstate.position)
        else {
            return Vec::new();
        };
        let (Some(local_paused), Some(local_position)) = (self.local_paused, self.local_position)
        else {
            return Vec::new();
        };

        let position_diff_seconds = (local_position - room_position).abs();
        let pause_matches = local_paused == room_paused;
        let position_tolerance_seconds =
            self.reconnect_state_restore_position_tolerance_seconds_effective();
        let position_matches = position_diff_seconds <= position_tolerance_seconds;
        if pause_matches && position_matches {
            self.reconnect_state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch = self
                .reconnect_state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch
                .saturating_add(1);
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles = 0;
            self.reset_reconnect_state_restore_correction_retry_exhaustions();
            self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining =
                0;
            self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
            self.clear_reconnect_state_restore_validation_state();
            return Vec::new();
        }

        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        let correction_suppressed_for_recovery_cycle =
            self.reconnect_state_restore_correction_recovery_suppressed_this_cycle;
        let correction_reenabled_for_this_cycle =
            self.reconnect_state_restore_correction_recovery_reenabled_this_cycle;
        if !self.reconnect_state_restore_validation_mismatch_seen_in_cycle
            && !correction_suppressed_for_recovery_cycle
        {
            self.reconnect_state_restore_validation_mismatch_seen_in_cycle = true;
            self.reconnect_state_restore_correction_metrics
                .mismatch_cycles_detected = self
                .reconnect_state_restore_correction_metrics
                .mismatch_cycles_detected
                .saturating_add(1);
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles = self
                .reconnect_state_restore_correction_consecutive_mismatch_cycles
                .saturating_add(1);
        }
        let consecutive_mismatch_cycles =
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles;
        let disable_after_mismatch_cycles = self
            .behavior_config
            .reconnect_state_restore_correction_disable_after_mismatch_cycles;
        let disable_correction_due_to_repeated_mismatches = matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches
        ) && disable_after_mismatch_cycles > 0
            && consecutive_mismatch_cycles >= disable_after_mismatch_cycles;
        let mut actions = Vec::new();
        let should_emit_mismatch_notification = !matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion
        ) && !disable_correction_due_to_repeated_mismatches;
        if correction_reenabled_for_this_cycle {
            self.reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles = self
                .reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
            ));
        }
        if should_emit_mismatch_notification
            && !self.reconnect_state_restore_validation_mismatch_notified
        {
            self.reconnect_state_restore_validation_mismatch_notified = true;
            self.reconnect_state_restore_correction_metrics
                .mismatch_notifications_emitted = self
                .reconnect_state_restore_correction_metrics
                .mismatch_notifications_emitted
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused,
                    room_paused,
                    local_position,
                    room_position,
                    position_diff_seconds,
                },
            ));
        }

        if correction_suppressed_for_recovery_cycle {
            self.reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles = self
                .reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: self
                        .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
                },
            ));
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if disable_correction_due_to_repeated_mismatches {
            if self.activate_reconnect_state_restore_correction_recovery_cooldown_if_configured() {
                self.reconnect_state_restore_correction_consecutive_mismatch_cycles = 0;
            }
            self.reconnect_state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches = self
                .reconnect_state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches
                .saturating_add(1);
            self.clear_reconnect_state_restore_validation_state();
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles,
                    disable_after_mismatch_cycles,
                },
            ));
            return actions;
        }

        if matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
        ) {
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if !pause_matches {
            actions.push(ClientRuntimeAction::SetPaused(room_paused));
        }
        if !position_matches {
            actions.push(ClientRuntimeAction::SetPosition(room_position));
        }
        actions
    }

    pub fn runtime_actions_for_reconnect_playlist_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let Some(restore_intent) = self.reconnect_playlist_restore_intent.take() else {
            return Vec::new();
        };
        if self.server_shared_playlists_supported == Some(false) {
            return Vec::new();
        }

        let mut actions = vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringPlaylist,
            ),
            ClientRuntimeAction::SetPlaylist {
                files: restore_intent.files,
            },
        ];
        if let Some(index) = restore_intent.index {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index });
        }
        actions
    }

    pub fn runtime_actions_for_controller_reidentify_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if self.server_managed_rooms_supported != Some(true) {
            self.controlled_room_switch_intent = None;
            self.controller_reidentify_intent = None;
            return Vec::new();
        }

        let mut actions = Vec::new();
        if let Some(room) = self.controlled_room_switch_intent.take() {
            actions.push(ClientRuntimeAction::SetRoom { room });
            actions.push(ClientRuntimeAction::RequestUserList);
        }
        if let Some((room, password)) = self.controller_reidentify_intent.take() {
            self.last_controller_auth_password_attempt = Some(password.clone());
            actions.push(ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ));
            actions.push(ClientRuntimeAction::RequestControllerAuth { room, password });
        }
        actions
    }

    pub fn runtime_actions_for_outbound_chat_message(
        &self,
        message: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.server_chat_supported != Some(true) {
            return Vec::new();
        }
        if self.chat_config.max_chat_message_length == 0 {
            return Vec::new();
        }
        let sanitized = Self::sanitize_chat_message_legacy_compatible(&message);
        let truncated = Self::truncate_chat_message_legacy_compatible(
            &sanitized,
            self.chat_config.max_chat_message_length,
        );
        vec![ClientRuntimeAction::SendChat { message: truncated }]
    }

    pub fn runtime_actions_for_local_ready_toggle(
        &self,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || self.server_readiness_supported != Some(true) {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetReady {
            ready: !self.local_user_ready(),
            manually_initiated,
        }]
    }

    pub fn runtime_actions_for_local_user_ready_set(
        &self,
        username: String,
        ready: bool,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        if username.is_empty() {
            if self.server_readiness_supported != Some(true) {
                return Vec::new();
            }
            return vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated,
            }];
        }
        if self.server_readiness_supported != Some(true)
            || self.server_set_others_readiness_supported != Some(true)
        {
            return Vec::new();
        }
        if self.local_can_control() != Some(true) {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetReadyForUser {
            ready,
            manually_initiated,
            username,
        }]
    }

    pub fn runtime_actions_for_local_controller_auth_request(
        &mut self,
        room: String,
        password: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        if self.server_managed_rooms_supported != Some(true) {
            return Vec::new();
        }
        if room.is_empty() {
            return Vec::new();
        }
        let password = Self::normalize_control_password_legacy_compatible(&password);
        self.last_controller_auth_password_attempt = Some(password.clone());
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ),
            ClientRuntimeAction::RequestControllerAuth { room, password },
        ]
    }

    pub fn runtime_actions_for_local_room_switch(
        &mut self,
        room: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.server_chat_supported.is_none() {
            return Vec::new();
        }
        let (room, inline_password) =
            Self::normalize_runtime_controlled_room_input_legacy_compatible(room);
        if room.is_empty() {
            return Vec::new();
        }
        if let Some(password) = inline_password.as_deref() {
            self.remember_control_password_for_room(&room, password);
        }
        let tracked_room = self
            .pending_local_room_switch_target
            .as_deref()
            .or(self.room.as_deref());
        if tracked_room != Some(room.as_str()) {
            self.pending_local_room_switch_target = Some(room.clone());
            self.reset_playlist_index_transition_tracking();
        }
        let mut actions = vec![
            ClientRuntimeAction::SetRoom { room: room.clone() },
            ClientRuntimeAction::RequestUserList,
        ];
        let controller_password = inline_password
            .filter(|password| !password.is_empty())
            .or_else(|| self.controlled_room_passwords.get(&room).cloned());
        if self.server_managed_rooms_supported == Some(true)
            && let Some(password) = controller_password
        {
            self.last_controller_auth_password_attempt = Some(password.clone());
            actions.push(ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ));
            actions.push(ClientRuntimeAction::RequestControllerAuth { room, password });
        }
        actions
    }

    pub fn local_room_command_target_with_legacy_fallback(&self, default_room: &str) -> String {
        let Some(username) = self.username.as_deref() else {
            return default_room.to_owned();
        };
        if let Some(room_name) = self
            .user_room(username)
            .filter(|room_name| !room_name.is_empty())
            .filter(|room_name| Self::is_controlled_room_name(room_name))
        {
            return room_name.to_owned();
        }
        self.user_file_name(username)
            .filter(|file_name| !file_name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| default_room.to_owned())
    }

    pub fn runtime_actions_for_local_pause_toggle(&mut self) -> Vec<ClientRuntimeAction> {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let target_paused = !self.effective_local_paused_state(now_seconds);
        self.runtime_actions_for_local_pause_change(target_paused, now_seconds)
    }

    pub fn runtime_actions_for_local_pause_set(
        &mut self,
        paused: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_local_pause_change(
            paused,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn runtime_actions_for_local_user_list_request(&self) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || self.server_chat_supported.is_none() {
            return Vec::new();
        }
        vec![ClientRuntimeAction::RequestUserList]
    }

    pub fn runtime_actions_for_local_playlist_index_set(
        &self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() || index < 0 {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Ok(index_usize) = usize::try_from(index) else {
            return Vec::new();
        };
        if index_usize >= playlist.files.len() {
            return Vec::new();
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[index_usize]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex { index }]
    }

    pub fn runtime_actions_for_local_playlist_next(&self) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        if playlist.files.is_empty() {
            return Vec::new();
        }
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };
        if current_index >= playlist.files.len() {
            return Vec::new();
        }
        if self.current_user_file_name() != Some(playlist.files[current_index].as_str()) {
            return Vec::new();
        }

        if playlist.files.len() == 1 {
            if !self.loop_single_files_enabled_legacy_compatible() {
                return Vec::new();
            }
            return vec![
                ClientRuntimeAction::SetPosition(0.0),
                ClientRuntimeAction::SetPaused(false),
            ];
        }

        let Some(next_index) = current_index.checked_add(1) else {
            return Vec::new();
        };
        if next_index >= playlist.files.len() {
            if !self.loop_at_end_of_playlist_enabled_legacy_compatible() {
                return Vec::new();
            }
            if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[0]) {
                return Vec::new();
            }
            return vec![ClientRuntimeAction::SetPlaylistIndex { index: 0 }];
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[next_index]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex {
            index: next_index as i64,
        }]
    }

    pub fn runtime_actions_for_local_playlist_queue(
        &mut self,
        file_name: String,
        select_after_queue: bool,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        if file_name.is_empty() {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        if current_files
            .iter()
            .any(|current_file| current_file == &file_name)
        {
            return Vec::new();
        }
        let mut files = current_files.clone();
        files.push(file_name);
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        let target_index = if select_after_queue {
            files.len().saturating_sub(1)
        } else {
            current_index
                .filter(|index| *index < current_files.len())
                .unwrap_or(0)
        };

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_delete(
        &mut self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() || index < 0 {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let current_files = playlist.files.clone();
        let current_index = playlist
            .index
            .and_then(|current| usize::try_from(current).ok());
        let Ok(delete_index) = usize::try_from(index) else {
            return Vec::new();
        };
        if delete_index >= current_files.len() {
            return Vec::new();
        }

        let mut files = current_files.clone();
        files.remove(delete_index);
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        if files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist { files }];
        }

        let target_index = current_index
            .map(|current| {
                if current < delete_index {
                    current
                } else if current > delete_index {
                    current.saturating_sub(1)
                } else {
                    delete_index.min(files.len().saturating_sub(1))
                }
            })
            .unwrap_or(0)
            .min(files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_replace(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        if files.iter().any(|file| file.is_empty()) {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        let playlist_changed = files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &files,
            );
        }
        if files.is_empty() {
            return playlist_changed
                .then_some(ClientRuntimeAction::SetPlaylist { files })
                .into_iter()
                .collect();
        }

        let target_index = selected_index
            .filter(|index| *index < files.len())
            .or_else(|| {
                Some(
                    Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                        &current_files,
                        current_index,
                        &files,
                    )
                    .min(files.len().saturating_sub(1)),
                )
            })
            .unwrap_or(0);

        let playlist_index_changed = current_index != Some(target_index);
        if !playlist_changed && !playlist_index_changed {
            return Vec::new();
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist { files });
        }
        if playlist_index_changed {
            actions.push(ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            });
        }
        actions
    }

    pub fn runtime_actions_for_local_playlist_undo(&mut self) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let Some(previous_files) = self.playlist_undo_snapshots.get(&room_name).cloned() else {
            return Vec::new();
        };
        if previous_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &previous_files,
        );

        if previous_files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            }];
        }

        let target_index =
            Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                &current_files,
                current_index,
                &previous_files,
            )
            .min(previous_files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_remaining(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_index >= current_files.len() {
            return Vec::new();
        }
        let shuffle_start = current_index.saturating_add(1);
        if shuffle_start >= current_files.len() {
            return Vec::new();
        }

        let mut shuffled_files = current_files.clone();
        let seed =
            self.next_playlist_shuffle_seed_legacy_compatible(&current_files, current_index, true);
        Self::shuffle_playlist_slice_in_place_legacy_compatible(
            &mut shuffled_files[shuffle_start..],
            seed,
        );
        if shuffled_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &shuffled_files,
        );
        vec![
            ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: current_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_entire(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.shared_playlist_runtime_commands_allowed_legacy_compatible() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_files.is_empty() {
            return Vec::new();
        }
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let mut shuffled_files = current_files.clone();
        let seed = self.next_playlist_shuffle_seed_legacy_compatible(
            &current_files,
            current_index.unwrap_or(0),
            false,
        );
        Self::shuffle_playlist_slice_in_place_legacy_compatible(&mut shuffled_files, seed);

        let playlist_changed = shuffled_files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &shuffled_files,
            );
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            });
        }
        if current_index != Some(0) || playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index: 0 });
        }
        actions
    }
}
