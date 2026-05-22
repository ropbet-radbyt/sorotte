use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::app) struct GuiSessionRoomPlaystate {
    pub(in crate::app) position_seconds: Option<f64>,
    pub(in crate::app) paused: Option<bool>,
    pub(in crate::app) do_seek: Option<bool>,
    pub(in crate::app) set_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiLocalPlayerUnpauseDecision {
    NotApplicable,
    Allow,
    Block,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiAttachedPlayerRuntimeAction {
    Paused(bool),
    Position(f64),
    PlaybackRate(f64),
}

#[allow(
    dead_code,
    reason = "Session adapter hooks are exercised by concrete adapters and targeted GUI tests."
)]
pub(in crate::app) trait GuiSessionRuntimeAdapter: Send {
    fn drain_gui_actions(&mut self, _state: &SorotteGuiShellAppState) -> Vec<GuiShellAction> {
        Vec::new()
    }

    fn playlist_control_available(&self) -> bool {
        false
    }

    fn adjust_command_availability(
        &self,
        _state: &SorotteGuiShellAppState,
        command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        command_availability
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn apply_message_json(&mut self, _json_line: &str) -> Result<(), String> {
        Err(
            "Attached session runtime does not accept inbound protocol transport messages."
                .to_owned(),
        )
    }

    fn set_room(&mut self, _room: String) -> Result<(), String> {
        Err("Attached session runtime does not support room changes.".to_owned())
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        self.set_room(default_room)
    }

    fn set_local_ready(&mut self, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support local readiness changes.".to_owned())
    }

    fn mark_local_media_opened_not_ready(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    fn set_user_ready(&mut self, _username: String, _ready: bool) -> Result<(), String> {
        Err("Attached session runtime does not support remote readiness changes.".to_owned())
    }

    fn request_controller_auth(&mut self, _room: String, _password: String) -> Result<(), String> {
        Err("Attached session runtime does not support controller auth requests.".to_owned())
    }

    fn queue_playlist_entry(
        &mut self,
        _entry: String,
        _select_after_queue: bool,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist queue operations."
                .to_owned(),
        )
    }

    fn set_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist selection changes."
                .to_owned(),
        )
    }

    fn advance_playlist_index(&mut self) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist advancement.".to_owned())
    }

    fn advance_playlist_index_attached_player_actions(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(Vec::new())
    }

    fn delete_playlist_index(&mut self, _index: usize) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist removal.".to_owned())
    }

    fn replace_playlist(
        &mut self,
        _files: Vec<String>,
        _selected_index: Option<usize>,
    ) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist reorder operations."
                .to_owned(),
        )
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        Err("Attached session runtime does not support shared playlist undo.".to_owned())
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        Err(
            "Attached session runtime does not support shared playlist shuffle operations."
                .to_owned(),
        )
    }

    fn sync_local_playback_telemetry(
        &mut self,
        _paused: Option<bool>,
        _position_seconds: Option<f64>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn sync_local_playback_cache_state(
        &mut self,
        _paused_for_cache: Option<bool>,
        _cache_buffering_percent: Option<f64>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn set_playback_paused(&mut self, _paused: bool) -> Result<bool, String> {
        Err("Attached session runtime does not support playback pause changes.".to_owned())
    }

    fn emit_immediate_playback_state_update(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    fn supports_playback_pause_changes(&self) -> bool {
        false
    }

    fn manual_seek_to_position_allowed(&self, _position_seconds: f64) -> Result<bool, String> {
        Ok(true)
    }

    fn record_manual_seek_to_position(&mut self, _position_seconds: f64) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek history.".to_owned())
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        Err("Attached session runtime does not support local seek undo.".to_owned())
    }

    fn pending_undo_seek_target_position(&self) -> Option<f64> {
        None
    }

    fn commit_undo_seek(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    fn local_position_seconds(&self) -> Option<f64> {
        None
    }

    fn local_pause_state(&self) -> Option<bool> {
        None
    }

    fn local_paused_for_cache(&self) -> Option<bool> {
        None
    }

    fn local_username(&self) -> Option<&str> {
        None
    }

    fn server_handshake_completed(&self) -> bool {
        true
    }

    fn current_room_playstate(&self) -> Option<GuiSessionRoomPlaystate> {
        None
    }

    fn current_room_playstate_for_attached_player_sync(&self) -> Option<GuiSessionRoomPlaystate> {
        self.current_room_playstate()
    }

    fn current_room_playlist_index(&self) -> Option<usize> {
        None
    }

    fn note_local_playlist_index_reset_intent(&mut self, _pause_before_sync: bool) {}

    fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        None
    }

    fn has_pending_playlist_index_reset_intent(&self) -> bool {
        false
    }

    fn can_auto_advance_to_next_playlist_item(&self) -> bool {
        false
    }

    fn set_autoplay_enabled(&mut self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, _threshold: usize) -> Result<(), String> {
        Ok(())
    }

    fn sync_runtime_settings(
        &mut self,
        _runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Result<(), String> {
        Ok(())
    }

    fn handle_local_player_unpause_attempt(
        &mut self,
    ) -> Result<GuiLocalPlayerUnpauseDecision, String> {
        Ok(GuiLocalPlayerUnpauseDecision::NotApplicable)
    }

    fn finalize_local_player_unpause_attempt(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn take_attached_player_local_runtime_actions(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(Vec::new())
    }

    fn attached_player_runtime_actions(
        &mut self,
        _now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(Vec::new())
    }

    fn publish_local_file_legacy_compatible(
        &mut self,
        _file_payload: &Value,
        _filename_privacy_mode: PrivacyMode,
        _filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), String> {
        Ok(())
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String>;

    fn attached_player_chat_input_ready(&self) -> bool {
        false
    }

    fn attached_player_chat_input_unavailable_message(&self) -> String {
        "Chat input from the attached player requires an active session with chat support."
            .to_owned()
    }

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String>;

    fn refresh_public_servers(
        &mut self,
        current_servers: Vec<(String, String)>,
        language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String>;

    fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        Err("Attached session runtime does not expose a missing-media search target.".to_owned())
    }

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String>;

    fn handle_transport_disconnect(
        &mut self,
        _now_seconds: f64,
        _retries: u32,
    ) -> Result<(), String> {
        Ok(())
    }

    fn drain_reconnect_delays(&mut self) -> Vec<f64> {
        Vec::new()
    }

    fn take_stop_reconnect_requested(&mut self) -> bool {
        false
    }

    fn prepare_for_transport_reconnect(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn disconnect_session(&mut self, _now_seconds: f64) -> Result<(), String> {
        Ok(())
    }
}
