use super::*;

impl GuiSessionRuntimeAdapter for GuiClientCoreChatSessionRuntimeAdapter {
    fn drain_gui_actions(&mut self, state: &SorotteGuiShellAppState) -> Vec<GuiShellAction> {
        self.drain_gui_actions_impl(state)
    }

    fn adjust_command_availability(
        &self,
        state: &SorotteGuiShellAppState,
        mut command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        let settings = state.configuration.to_stored_settings();
        if !legacy_chat_input_enabled(&settings) || state.pending_operation.is_some() {
            return command_availability;
        }
        match self.runtime.session().server_chat_supported() {
            Some(true) => {
                command_availability.can_send_chat_message = true;
                command_availability.chat_unavailable_reason = None;
            }
            None => {
                command_availability.can_send_chat_message = false;
                command_availability.chat_unavailable_reason = Some(
                    "Chat input is unavailable until the server Hello confirms chat support."
                        .to_owned(),
                );
            }
            Some(false) => {
                command_availability.can_send_chat_message = false;
                command_availability.chat_unavailable_reason =
                    Some("Chat input is unavailable because the server disabled chat.".to_owned());
            }
        }
        command_availability
    }

    fn playlist_control_available(&self) -> bool {
        self.shared_playlist_control_available()
    }

    fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        GuiClientCoreChatSessionRuntimeAdapter::flush_outbound_protocol_lines(self)
    }

    fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        GuiClientCoreChatSessionRuntimeAdapter::apply_message_json(self, json_line)
    }

    fn set_room(&mut self, room: String) -> Result<(), String> {
        match self.runtime.run_set_room(room) {
            Ok(true) => {
                self.pending_room_for_next_hello =
                    self.latest_outbound_room_target_for_next_hello();
                Ok(())
            }
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn set_room_with_legacy_fallback(&mut self, default_room: String) -> Result<(), String> {
        match self.runtime.run_set_room_with_legacy_fallback(default_room) {
            Ok(true) => {
                self.pending_room_for_next_hello =
                    self.latest_outbound_room_target_for_next_hello();
                Ok(())
            }
            Ok(false) => {
                if self.runtime.session().server_chat_supported().is_none() {
                    Err(
                        "Client-core session runtime cannot change rooms until the server Hello completes."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound room change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime room change dispatch failed: {error}"
            )),
        }
    }

    fn send_chat_message(&mut self, message: String) -> Result<(), String> {
        match self.runtime.run_send_chat_message(message) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_chat_supported() {
                None => Err(
                    "Client-core session runtime cannot send chat until the server Hello enables chat."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot send chat because the server disabled chat."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound chat message."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime chat dispatch failed: {error}"
            )),
        }
    }

    fn attached_player_chat_input_ready(&self) -> bool {
        self.runtime.session().server_chat_supported() == Some(true)
    }

    fn attached_player_chat_input_unavailable_message(&self) -> String {
        match self.runtime.session().server_chat_supported() {
            None => {
                "Chat input from the attached player cannot be sent until the server Hello enables chat."
                    .to_owned()
            }
            Some(false) => {
                "Chat input from the attached player cannot be sent because the server disabled chat."
                    .to_owned()
            }
            Some(true) => {
                "Chat input from the attached player could not be sent because the session runtime is not ready."
                    .to_owned()
            }
        }
    }

    fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user("", ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change readiness until the server Hello enables readiness."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change readiness because the server disabled readiness."
                        .to_owned(),
                ),
                Some(true) => Err(
                    "Client-core session runtime did not queue an outbound readiness change."
                        .to_owned(),
                ),
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn mark_local_media_opened_not_ready(&mut self) -> Result<bool, String> {
        self.runtime
            .run_local_media_opened_not_ready()
            .map_err(|error| {
                format!(
                    "Client-core session runtime local media-open readiness dispatch failed: {error}"
                )
            })
    }

    fn set_user_ready(&mut self, username: String, ready: bool) -> Result<(), String> {
        match self.runtime.run_set_ready_for_user(username, ready, true) {
            Ok(true) => Ok(()),
            Ok(false) => match self.runtime.session().server_set_others_readiness_supported() {
                None => Err(
                    "Client-core session runtime cannot change other users' readiness until the server Hello enables remote readiness changes."
                        .to_owned(),
                ),
                Some(false) => Err(
                    "Client-core session runtime cannot change other users' readiness because the server disabled remote readiness changes."
                        .to_owned(),
                ),
                Some(true) => {
                    if self.runtime.session().local_can_control() != Some(true) {
                        Err(
                            "Client-core session runtime cannot change other users' readiness because the local user cannot control the current room."
                                .to_owned(),
                        )
                    } else {
                        Err(
                            "Client-core session runtime did not queue an outbound remote readiness change."
                                .to_owned(),
                        )
                    }
                }
            },
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    fn request_controller_auth(&mut self, room: String, password: String) -> Result<(), String> {
        match self.runtime.run_request_controller_auth(room, password) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.runtime.session().username.is_none() {
                    Err(
                        "Client-core session runtime cannot request controller access until the server Hello is received."
                            .to_owned(),
                    )
                } else if self
                    .runtime
                    .session()
                    .server_managed_rooms_supported()
                    .is_none()
                {
                    Err(
                        "Client-core session runtime cannot request controller access until the server Hello enables controlled-room support."
                            .to_owned(),
                    )
                } else if self.runtime.session().server_managed_rooms_supported() == Some(false) {
                    Err(
                        "Client-core session runtime cannot request controller access because the server disabled controlled-room support."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound controller-auth request."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime controller-auth dispatch failed: {error}"
            )),
        }
    }

    fn queue_playlist_entry(
        &mut self,
        entry: String,
        select_after_queue: bool,
    ) -> Result<(), String> {
        if self.projected_current_room_playlist_contains_entry(&entry) {
            return Ok(());
        }
        match self
            .runtime
            .run_queue_playlist_item(entry.clone(), select_after_queue)
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self.projected_current_room_playlist_contains_entry(&entry) {
                    return Ok(());
                }
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist entry."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist queue dispatch failed: {error}"
            )),
        }
    }

    fn set_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_set_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot change the shared playlist selection before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist selection change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist selection dispatch failed: {error}"
            )),
        }
    }

    fn advance_playlist_index(&mut self) -> Result<(), String> {
        match self.runtime.run_advance_playlist_index() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot advance the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist advancement."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist advancement dispatch failed: {error}"
            )),
        }
    }

    fn advance_playlist_index_attached_player_actions(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        let actions = self
            .runtime
            .session()
            .runtime_actions_for_local_playlist_next();
        if actions
            .iter()
            .any(|action| matches!(action, ClientRuntimeAction::SetPlaylistIndex { .. }))
        {
            return Ok(Vec::new());
        }
        Ok(actions
            .into_iter()
            .filter_map(|action| match action {
                ClientRuntimeAction::SetPaused(paused) => {
                    Some(GuiAttachedPlayerRuntimeAction::Paused(paused))
                }
                ClientRuntimeAction::SetPosition(position_seconds) => {
                    Some(GuiAttachedPlayerRuntimeAction::Position(position_seconds))
                }
                ClientRuntimeAction::SetPlaybackRate(playback_rate) => {
                    Some(GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate))
                }
                _ => None,
            })
            .collect())
    }

    fn delete_playlist_index(&mut self, index: usize) -> Result<(), String> {
        let Ok(index) = i64::try_from(index) else {
            return Err("Requested shared playlist index exceeds the supported range.".to_owned());
        };
        match self.runtime.run_delete_playlist_index(index) {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot remove shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist removal."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist removal dispatch failed: {error}"
            )),
        }
    }

    fn replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<(), String> {
        match self
            .runtime
            .run_replace_playlist(files.clone(), selected_index)
        {
            Ok(true) => {
                self.set_optimistic_current_room_playlist(files, selected_index);
                Ok(())
            }
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot reorder the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist reorder."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime playlist reorder dispatch failed: {error}"
            )),
        }
    }

    fn undo_playlist_change(&mut self) -> Result<(), String> {
        match self.runtime.run_undo_playlist_change() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot undo shared playlist changes before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist undo."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist undo dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_remaining_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle remaining shared playlist entries before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
        match self.runtime.run_shuffle_entire_playlist() {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.shared_playlist_control_available() {
                    Err(
                        "Client-core session runtime cannot shuffle the shared playlist before room control becomes available."
                            .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue a shared playlist shuffle."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime shared playlist shuffle dispatch failed: {error}"
            )),
        }
    }

    fn sync_local_playback_telemetry(
        &mut self,
        paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .apply_player_playback_telemetry_update(&PlayerPlaybackTelemetryUpdate {
                paused,
                position_seconds,
                playback_rate: None,
                paused_for_cache: None,
                cache_buffering_percent: None,
            });
        Ok(())
    }

    fn sync_local_playback_cache_state(
        &mut self,
        paused_for_cache: Option<bool>,
        cache_buffering_percent: Option<f64>,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .apply_player_playback_telemetry_update(&PlayerPlaybackTelemetryUpdate {
                paused: None,
                position_seconds: None,
                playback_rate: None,
                paused_for_cache,
                cache_buffering_percent,
            });
        Ok(())
    }

    fn set_playback_paused(&mut self, paused: bool) -> Result<bool, String> {
        match self.runtime.run_set_paused(paused) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime playback pause dispatch failed: {error}"
            )),
        }
    }

    fn emit_immediate_playback_state_update(&mut self) -> Result<bool, String> {
        Ok(self
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(self.dont_slow_down_with_me))
    }

    fn supports_playback_pause_changes(&self) -> bool {
        true
    }

    fn manual_seek_to_position_allowed(&self, position_seconds: f64) -> Result<bool, String> {
        Ok(self
            .runtime
            .session()
            .local_seek_target_allowed(position_seconds, system_time_seconds()))
    }

    fn record_manual_seek_to_position(&mut self, position_seconds: f64) -> Result<bool, String> {
        match self.runtime.run_seek_to_position(position_seconds) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime seek dispatch failed: {error}"
            )),
        }
    }

    fn undo_seek(&mut self) -> Result<bool, String> {
        match self.runtime.run_undo_seek() {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime undo-seek dispatch failed: {error}"
            )),
        }
    }

    fn pending_undo_seek_target_position(&self) -> Option<f64> {
        self.runtime
            .session()
            .last_seek_position_before_manual_seek()
    }

    fn commit_undo_seek(&mut self) -> Result<bool, String> {
        self.undo_seek()
    }

    fn local_position_seconds(&self) -> Option<f64> {
        self.runtime.session().local_position_seconds()
    }

    fn local_pause_state(&self) -> Option<bool> {
        self.runtime.session().local_paused()
    }

    fn local_paused_for_cache(&self) -> Option<bool> {
        self.runtime.session().local_paused_for_cache()
    }

    fn local_username(&self) -> Option<&str> {
        self.runtime.session().username.as_deref()
    }

    fn server_handshake_completed(&self) -> bool {
        self.runtime.session().server_chat_supported().is_some()
    }

    fn current_room_playstate(&self) -> Option<GuiSessionRoomPlaystate> {
        self.runtime
            .session()
            .current_room_playstate()
            .map(|playstate| GuiSessionRoomPlaystate {
                position_seconds: playstate.position,
                paused: playstate.paused,
                do_seek: playstate.do_seek,
                set_by: playstate.set_by.clone(),
            })
    }

    fn current_room_playstate_for_attached_player_sync(&self) -> Option<GuiSessionRoomPlaystate> {
        if !self
            .runtime
            .session()
            .current_room_playstate_has_remote_authority()
        {
            return None;
        }
        self.runtime
            .current_room_playstate_legacy_ping_compatible_now()
            .map(|playstate| GuiSessionRoomPlaystate {
                position_seconds: playstate.position,
                paused: playstate.paused,
                do_seek: playstate.do_seek,
                set_by: playstate.set_by,
            })
    }

    fn current_room_playlist_index(&self) -> Option<usize> {
        self.projected_current_room_playlist()
            .and_then(|playlist| playlist.index)
            .and_then(|index| usize::try_from(index).ok())
    }

    fn note_local_playlist_index_reset_intent(&mut self, pause_before_sync: bool) {
        self.runtime
            .session_mut()
            .begin_local_playlist_index_reset_intent(pause_before_sync, system_time_seconds());
    }

    fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        self.runtime
            .session_mut()
            .take_pending_playlist_index_reset_intent()
    }

    fn has_pending_playlist_index_reset_intent(&self) -> bool {
        self.runtime
            .session()
            .has_pending_playlist_index_reset_intent()
    }

    fn can_auto_advance_to_next_playlist_item(&self) -> bool {
        !self
            .runtime
            .session()
            .runtime_actions_for_local_playlist_next()
            .is_empty()
    }

    fn set_autoplay_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.runtime.session_mut().set_autoplay_enabled(enabled);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn set_autoplay_threshold(&mut self, threshold: usize) -> Result<(), String> {
        self.runtime
            .session_mut()
            .readiness_autoplay_config_mut()
            .auto_play_threshold = Some(threshold);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn set_strong_same_media_match_satisfies_filename_gate(
        &mut self,
        satisfied: bool,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .set_strong_same_media_match_satisfies_filename_gate(satisfied);
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn sync_runtime_settings(
        &mut self,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Result<(), String> {
        self.apply_runtime_settings_snapshot(runtime_settings);
        Ok(())
    }

    fn handle_local_player_unpause_attempt(
        &mut self,
    ) -> Result<GuiLocalPlayerUnpauseDecision, String> {
        if self
            .current_room_playstate_for_attached_player_sync()
            .and_then(|playstate| playstate.paused)
            != Some(true)
        {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        }

        let readiness_supported = self
            .runtime
            .session()
            .server_readiness_supported()
            .unwrap_or(false);
        if !readiness_supported {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        }

        let local_can_control = self.runtime.session().local_can_control().unwrap_or(false);
        let is_playing_music = self.runtime.session().is_playing_music();
        if self
            .runtime
            .session()
            .instaplay_conditions_met(local_can_control, is_playing_music)
        {
            return Ok(GuiLocalPlayerUnpauseDecision::Allow);
        }

        self.runtime
            .run_readiness_unpause_attempt(
                system_time_seconds(),
                readiness_supported,
                local_can_control,
                is_playing_music,
            )
            .map_err(|error| {
                format!("Client-core session runtime readiness/unpause dispatch failed: {error}")
            })?;
        Ok(GuiLocalPlayerUnpauseDecision::Block)
    }

    fn finalize_local_player_unpause_attempt(&mut self) -> Result<(), String> {
        if self
            .current_room_playstate_for_attached_player_sync()
            .and_then(|playstate| playstate.paused)
            != Some(true)
        {
            return Ok(());
        }

        let readiness_supported = self
            .runtime
            .session()
            .server_readiness_supported()
            .unwrap_or(false);
        if !readiness_supported {
            return Ok(());
        }

        let local_can_control = self.runtime.session().local_can_control().unwrap_or(false);
        let is_playing_music = self.runtime.session().is_playing_music();
        self.runtime
            .run_readiness_unpause_attempt(
                system_time_seconds(),
                readiness_supported,
                local_can_control,
                is_playing_music,
            )
            .map_err(|error| {
                format!("Client-core session runtime readiness/unpause dispatch failed: {error}")
            })
    }

    fn take_attached_player_local_runtime_actions(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(std::mem::take(
            &mut self.pending_attached_player_local_runtime_actions,
        ))
    }

    fn attached_player_runtime_actions(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        let Some(room_playstate) = self
            .runtime
            .current_room_playstate_legacy_ping_compatible_now()
        else {
            return Ok(Vec::new());
        };
        let Some(local_position) = self.runtime.session().local_position_seconds() else {
            return Ok(Vec::new());
        };

        let local_can_control = self.runtime.session().local_can_control().unwrap_or(false);
        let actions = self
            .runtime
            .session_mut()
            .runtime_actions_for_desync_correction_against_room_playstate(
                RoomPlaystateView {
                    position: room_playstate.position,
                    paused: room_playstate.paused,
                    do_seek: room_playstate.do_seek,
                    set_by: room_playstate.set_by.clone(),
                },
                now_seconds,
                local_position,
                local_can_control,
                self.dont_slow_down_with_me,
                true,
            );
        Ok(actions
            .into_iter()
            .filter_map(|action| match action {
                ClientRuntimeAction::SetPosition(position_seconds) => {
                    Some(GuiAttachedPlayerRuntimeAction::Position(position_seconds))
                }
                ClientRuntimeAction::SetPlaybackRate(playback_rate) => {
                    Some(GuiAttachedPlayerRuntimeAction::PlaybackRate(playback_rate))
                }
                _ => None,
            })
            .collect())
    }

    fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), String> {
        self.runtime
            .publish_local_file_legacy_compatible(
                file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            )
            .map_err(|error| {
                format!("Client-core session runtime local file publish failed: {error}")
            })
    }

    fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        let Some((_label, address)) = selected_server else {
            return Err(
                "Client-core session runtime cannot connect because no public server is selected."
                    .to_owned(),
            );
        };
        let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
        if host.trim().is_empty() {
            return Err(
                "Client-core session runtime cannot connect because the selected public-server address is invalid."
                    .to_owned(),
            );
        }
        self.reset_session_for_reconnect();
        Ok(())
    }

    fn refresh_public_servers(
        &mut self,
        _current_servers: Vec<(String, String)>,
        _language: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(refreshed_servers) = Self::refreshed_public_server_rows_from_env()? {
            return Ok(refreshed_servers);
        }
        #[cfg(test)]
        {
            Ok(Self::normalize_public_server_rows(_current_servers))
        }
        #[cfg(not(test))]
        {
            let refreshed_servers = remote_services::fetch_public_servers(_language)?;
            Ok(Self::normalize_public_server_rows(refreshed_servers))
        }
    }

    fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        GuiClientCoreChatSessionRuntimeAdapter::missing_media_search_target_file_name(self)
    }

    fn search_missing_media(&mut self, directories: Vec<String>) -> Result<Option<String>, String> {
        let target_file_name = self.missing_media_search_target_file_name()?;
        for directory in directories {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                Self::search_path_for_missing_media_target(&target_file_name, Path::new(trimmed))?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    fn handle_transport_disconnect(
        &mut self,
        now_seconds: f64,
        retries: u32,
    ) -> Result<(), String> {
        self.runtime
            .run_disconnect(now_seconds)
            .map_err(|error| format!("Client-core session runtime disconnect failed: {error}"))?;
        self.runtime
            .run_reconnect_retry(retries)
            .map_err(|error| format!("Client-core session runtime reconnect retry failed: {error}"))
    }

    fn drain_reconnect_delays(&mut self) -> Vec<f64> {
        self.runtime.drain_reconnect_requests()
    }

    fn take_stop_reconnect_requested(&mut self) -> bool {
        self.runtime.take_stop_reconnect_requested()
    }

    fn prepare_for_transport_reconnect(&mut self) -> Result<(), String> {
        self.prepare_transport_reconnect();
        Ok(())
    }

    fn disconnect_session(&mut self, now_seconds: f64) -> Result<(), String> {
        self.runtime
            .run_disconnect(now_seconds)
            .map_err(|error| format!("Client-core session runtime disconnect failed: {error}"))
    }
}
