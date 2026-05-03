use super::*;

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientRuntimeControl,
{
    pub fn run_send_chat_message(
        &mut self,
        message: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        if self.session.server_chat_supported().is_none() {
            return Ok(false);
        }
        let actions = self
            .session
            .runtime_actions_for_outbound_chat_message(message.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_player_chat_input_if_needed(&mut self) -> Result<usize, PlayerError> {
        let mut sent = 0usize;
        while let Some(message) = self.player.take_pending_chat_request() {
            if self.run_send_chat_message(message)? {
                sent += 1;
            }
        }
        Ok(sent)
    }

    pub fn run_toggle_ready(&mut self, manually_initiated: bool) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_ready_toggle(manually_initiated);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_ready_for_user(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_ready_set(
            username.into(),
            ready,
            manually_initiated,
        );
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_local_media_opened_not_ready(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_media_opened_not_ready();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_request_controller_auth(
        &mut self,
        room: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_controller_auth_request(room.into(), password.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room(&mut self, room: impl Into<String>) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_room_switch(room.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room_with_legacy_fallback(
        &mut self,
        default_room: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let default_room = default_room.into();
        let room = self
            .session
            .local_room_command_target_with_legacy_fallback(&default_room);
        self.run_set_room(room)
    }

    pub fn run_toggle_pause(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_pause_toggle();
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_set_paused(&mut self, paused: bool) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_pause_set(paused);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_request_user_list(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_list_request();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_index_set(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        self.finalize_local_playlist_index_switch_if_needed(&actions);
        Ok(sent)
    }

    pub fn run_advance_playlist_index(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_next();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        self.finalize_local_playlist_index_switch_if_needed(&actions);
        Ok(sent)
    }

    pub fn run_queue_playlist_item(
        &mut self,
        file_name: impl Into<String>,
        select_after_queue: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_queue(file_name.into(), select_after_queue);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_delete_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_delete(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_replace(files, selected_index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_undo_playlist_change(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_undo();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_shuffle_remaining_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_remaining();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_shuffle_entire_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_entire();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_seek_to_position(&mut self, target_position: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek(target_position);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_seek_by_offset(&mut self, offset_seconds: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self
            .session
            .runtime_actions_for_local_seek_offset(offset_seconds);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_undo_seek(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek_undo();
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_disconnect(&mut self, now_seconds: f64) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.handle_disconnect(now_seconds);
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_file_publish_legacy_compatible(
                file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn publish_pending_local_file_update_legacy_compatible(
        &mut self,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<bool, PlayerError> {
        let Some(local_file_update) = self.player.take_local_file_update() else {
            return Ok(false);
        };

        let file_payload = Self::local_file_update_payload(&local_file_update);
        self.last_local_file_update = Some(local_file_update.clone());
        self.publish_local_file_legacy_compatible(
            &file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        )?;
        Ok(true)
    }

    pub(crate) fn local_file_update_payload(local_file_update: &LocalFileUpdate) -> Value {
        let mut payload = Map::new();
        payload.insert(
            "name".to_owned(),
            Value::String(local_file_update.name.clone()),
        );
        if let Some(duration_seconds) = local_file_update.duration_seconds {
            payload.insert("duration".to_owned(), Value::from(duration_seconds));
        }
        if let Some(size_bytes) = local_file_update.size_bytes {
            payload.insert("size".to_owned(), Value::from(size_bytes));
        }
        if let Some(path) = local_file_update.path.as_ref() {
            payload.insert("path".to_owned(), Value::String(path.clone()));
        }
        Value::Object(payload)
    }

    pub(crate) fn sync_player_playback_telemetry_into_session_and_buffer(&mut self) {
        while let Some(update) = self.player.take_playback_telemetry_update() {
            self.session.apply_player_playback_telemetry_update(&update);
            self.pending_player_playback_telemetry_updates.push(update);
        }
    }
}
