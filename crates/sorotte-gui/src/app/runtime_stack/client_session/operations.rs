use super::*;
#[cfg(test)]
use crate::app::runtime_stack::test_support::{SessionFailurePoint, SessionObservation};
use crate::app::runtime_state::GuiRuntimeState;

fn gui_actions_from_playback_coordinator(
    actions: Vec<PlaybackCoordinatorAction>,
) -> Vec<GuiAttachedPlayerRuntimeAction> {
    actions
        .into_iter()
        .filter_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command,
            } => Some(GuiAttachedPlayerRuntimeAction::Coordinator {
                command_id,
                command,
            }),
            PlaybackCoordinatorAction::RequestRoomPause { .. }
            | PlaybackCoordinatorAction::RevisionApplied { .. }
            | PlaybackCoordinatorAction::Started { .. }
            | PlaybackCoordinatorAction::Degraded { .. }
            | PlaybackCoordinatorAction::CommandTimedOut { .. } => None,
        })
        .collect()
}

impl GuiClientSession {
    pub(in crate::app) fn drain_gui_actions(
        &mut self,
        state: &GuiRuntimeState,
    ) -> Vec<GuiShellAction> {
        self.drain_gui_actions_impl(state)
    }

    pub(in crate::app) fn adjust_command_availability(
        &self,
        state: &GuiRuntimeState,
        mut command_availability: GuiCommandAvailabilityState,
    ) -> GuiCommandAvailabilityState {
        if !chat_input_enabled(&self.runtime_settings.settings)
            || state.session.pending_operation.is_some()
        {
            return command_availability;
        }
        let session = self.runtime.session();
        if !session.is_active() {
            command_availability.can_send_chat_message = false;
            command_availability.chat_unavailable_reason = Some(
                "Chat input is unavailable until the server Hello confirms chat support."
                    .to_owned(),
            );
        } else if session.server_chat_supported() {
            command_availability.can_send_chat_message = true;
            command_availability.chat_unavailable_reason = None;
        } else {
            command_availability.can_send_chat_message = false;
            command_availability.chat_unavailable_reason =
                Some("Chat input is unavailable because the server disabled chat.".to_owned());
        }
        command_availability
    }

    pub(in crate::app) fn playlist_control_available(&self) -> bool {
        self.shared_playlist_control_available()
    }

    pub(in crate::app) fn current_room_playlist_revision(&self) -> Option<u64> {
        self.projected_current_room_playlist()
            .map(|playlist| playlist.revision)
    }

    pub(in crate::app) fn current_room_playlist_selection_revision(&self) -> Option<u64> {
        self.runtime
            .session()
            .current_room_playlist_selection_revision()
    }

    pub(in crate::app) fn current_room_playlist_remote_revision(&self) -> u64 {
        self.runtime
            .session()
            .current_room_playlist_remote_revision()
    }

    pub(in crate::app) fn apply_message_json_at(
        &mut self,
        json_line: &str,
        received_at_seconds: f64,
    ) -> Result<(), String> {
        let inbound_paused = serde_json::from_str::<serde_json::Value>(json_line)
            .ok()
            .and_then(|message| {
                message
                    .get("State")?
                    .get("playstate")?
                    .get("paused")?
                    .as_bool()
            });
        if let Some(target_paused) = inbound_paused {
            let before = self.runtime.playback_coordination_snapshot();
            crate::app::test_lifecycle::record_playback_control(
                "playback-control-canonical-inbound-before",
                crate::app::test_lifecycle::PlaybackControlObservation {
                    target_paused: Some(target_paused),
                    current_room_paused: self
                        .runtime
                        .session()
                        .current_room_playstate()
                        .and_then(|playstate| playstate.paused),
                    media_generation: before.media_generation,
                    pending_local_pause_intent: before.pending_local_pause_intent,
                    pending_local_pause_intent_dormant: before.pending_local_pause_intent_dormant,
                    last_local_pause_intent_stage_accepted: before
                        .last_local_pause_intent_stage_accepted,
                    transport_telemetry_observed: before.transport_telemetry_observed,
                    ordinary_correction_blocked: before.ordinary_correction_blocked,
                    playlist_reset_pending: self
                        .runtime
                        .session()
                        .has_pending_playlist_index_reset_intent(),
                    state_queued: None,
                },
            );
        }
        let result =
            GuiClientSession::apply_inbound_message_json_at(self, json_line, received_at_seconds);
        if let Some(target_paused) = inbound_paused {
            let after = self.runtime.playback_coordination_snapshot();
            crate::app::test_lifecycle::record_playback_control(
                "playback-control-canonical-inbound-after",
                crate::app::test_lifecycle::PlaybackControlObservation {
                    target_paused: Some(target_paused),
                    current_room_paused: self
                        .runtime
                        .session()
                        .current_room_playstate()
                        .and_then(|playstate| playstate.paused),
                    media_generation: after.media_generation,
                    pending_local_pause_intent: after.pending_local_pause_intent,
                    pending_local_pause_intent_dormant: after.pending_local_pause_intent_dormant,
                    last_local_pause_intent_stage_accepted: after
                        .last_local_pause_intent_stage_accepted,
                    transport_telemetry_observed: after.transport_telemetry_observed,
                    ordinary_correction_blocked: after.ordinary_correction_blocked,
                    playlist_reset_pending: self
                        .runtime
                        .session()
                        .has_pending_playlist_index_reset_intent(),
                    state_queued: Some(result.is_ok()),
                },
            );
        }
        result
    }

    pub(in crate::app) fn set_room(&mut self, room: String) -> Result<(), String> {
        match self.dispatch_application_command(ClientCommand::SetRoom {
            room,
            default_room_fallback: false,
        }) {
            Ok(true) => {
                self.pending_room_for_next_hello =
                    self.latest_outbound_room_target_for_next_hello();
                Ok(())
            }
            Ok(false) => {
                if !self.runtime.session().is_active() {
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

    pub(in crate::app) fn set_room_with_default_fallback(
        &mut self,
        default_room: String,
    ) -> Result<(), String> {
        match self.dispatch_application_command(ClientCommand::SetRoom {
            room: default_room,
            default_room_fallback: true,
        }) {
            Ok(true) => {
                self.pending_room_for_next_hello =
                    self.latest_outbound_room_target_for_next_hello();
                Ok(())
            }
            Ok(false) => {
                if !self.runtime.session().is_active() {
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

    pub(in crate::app) fn send_chat_message(&mut self, message: String) -> Result<(), String> {
        match self.dispatch_application_command(ClientCommand::SendChat(message)) {
            Ok(true) => Ok(()),
            Ok(false) => {
                let session = self.runtime.session();
                if !session.is_active() {
                    Err(
                    "Client-core session runtime cannot send chat until the server Hello enables chat."
                        .to_owned(),
                    )
                } else if !session.server_chat_supported() {
                    Err(
                    "Client-core session runtime cannot send chat because the server disabled chat."
                        .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound chat message."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime chat dispatch failed: {error}"
            )),
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn queue_user_list_for_test(&mut self) -> Result<bool, String> {
        self.dispatch_application_command(ClientCommand::RequestUserList)
            .map_err(|error| {
                format!("Client-core session runtime user-list dispatch failed: {error}")
            })
    }

    pub(in crate::app) fn attached_player_chat_input_ready(&self) -> bool {
        self.runtime.session().server_chat_supported()
    }

    pub(in crate::app) fn attached_player_chat_input_unavailable_message(&self) -> String {
        let session = self.runtime.session();
        if !session.is_active() {
            "Chat input from the attached player cannot be sent until the server Hello enables chat."
                .to_owned()
        } else if !session.server_chat_supported() {
            "Chat input from the attached player cannot be sent because the server disabled chat."
                .to_owned()
        } else {
            "Chat input from the attached player could not be sent because the session runtime is not ready."
                .to_owned()
        }
    }

    pub(in crate::app) fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
        match self.dispatch_application_command(ClientCommand::SetReadyFrom {
            username: None,
            ready: Some(ready),
            manually_initiated: true,
            surface: sorotte_protocol::DirectReadinessSurface::GuiButton,
        }) {
            Ok(true) => Ok(()),
            Ok(false) => {
                let session = self.runtime.session();
                if !session.is_active() {
                    Err(
                    "Client-core session runtime cannot change readiness until the server Hello enables readiness."
                        .to_owned(),
                    )
                } else if !session.server_readiness_supported() {
                    Err(
                    "Client-core session runtime cannot change readiness because the server disabled readiness."
                        .to_owned(),
                    )
                } else {
                    Err(
                        "Client-core session runtime did not queue an outbound readiness change."
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    pub(in crate::app) fn set_user_ready(
        &mut self,
        username: String,
        ready: bool,
    ) -> Result<(), String> {
        match self.dispatch_application_command(ClientCommand::SetReadyFrom {
            username: Some(username),
            ready: Some(ready),
            manually_initiated: true,
            surface: sorotte_protocol::DirectReadinessSurface::GuiButton,
        }) {
            Ok(true) => Ok(()),
            Ok(false) => {
                let session = self.runtime.session();
                if !session.is_active() {
                    Err(
                    "Client-core session runtime cannot change other users' readiness until the server Hello enables remote readiness changes."
                        .to_owned(),
                    )
                } else if !session.server_set_others_readiness_supported() {
                    Err(
                    "Client-core session runtime cannot change other users' readiness because the server disabled remote readiness changes."
                        .to_owned(),
                    )
                } else if session.local_can_control() != Some(true) {
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
            Err(error) => Err(format!(
                "Client-core session runtime readiness dispatch failed: {error}"
            )),
        }
    }

    pub(in crate::app) fn request_controller_auth(
        &mut self,
        room: String,
        password: String,
    ) -> Result<(), String> {
        match self
            .dispatch_application_command(ClientCommand::request_controller_auth(room, password))
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                if !self.runtime.session().is_active() {
                    Err(
                        "Client-core session runtime cannot request controller access until the server Hello is received."
                            .to_owned(),
                    )
                } else if !self.runtime.session().server_managed_rooms_supported() {
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

    pub(in crate::app) fn queue_playlist_entry(
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

    pub(in crate::app) fn queue_playlist_entry_with_delivery_fence(
        &mut self,
        entry: String,
        select_after_queue: bool,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.queue_playlist_entry(entry, select_after_queue)?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn set_playlist_index(&mut self, index: usize) -> Result<(), String> {
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

    pub(in crate::app) fn set_playlist_index_with_delivery_fence(
        &mut self,
        index: usize,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.set_playlist_index(index)?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn advance_playlist_index(&mut self) -> Result<(), String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PlaylistAdvance);
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

    pub(in crate::app) fn advance_playlist_index_with_delivery_fence(
        &mut self,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.advance_playlist_index()?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn has_pending_natural_playback_completion(&self) -> bool {
        self.runtime.has_pending_natural_playback_completion()
    }

    pub(in crate::app) fn invalidate_pending_natural_playback_completion(&mut self) {
        self.runtime
            .invalidate_pending_natural_playback_completion();
    }

    pub(in crate::app) fn advance_after_natural_completion_with_delivery_fence(
        &mut self,
    ) -> Result<Option<GuiPlaylistProtocolDeliveryFence>, String> {
        let advanced = self
            .runtime
            .run_advance_playlist_after_natural_completion()
            .map_err(|error| format!("Natural playlist completion dispatch failed: {error}"))?;
        if !advanced {
            return Ok(None);
        }
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PlaylistAdvance);
        self.pending_playlist_protocol_delivery_fence().map(Some)
    }

    pub(in crate::app) fn advance_playlist_index_attached_player_actions(
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
                    Some(GuiAttachedPlayerRuntimeAction::Paused {
                        paused,
                        cause: PlayerCommandCause::PlaylistTransition,
                    })
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

    pub(in crate::app) fn delete_playlist_index(&mut self, index: usize) -> Result<(), String> {
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

    pub(in crate::app) fn delete_playlist_index_with_delivery_fence(
        &mut self,
        index: usize,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.delete_playlist_index(index)?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<(), String> {
        let already_matches_projection =
            self.projected_current_room_playlist()
                .is_some_and(|playlist| {
                    playlist.files.as_slice() == files.as_slice()
                        && selected_index.is_none_or(|requested_index| {
                            i64::try_from(requested_index).is_ok_and(|requested_index| {
                                playlist.index == Some(requested_index)
                            })
                        })
                });
        if already_matches_projection && self.shared_playlist_control_available() {
            return Ok(());
        }

        match self.runtime.run_replace_playlist(files, selected_index) {
            Ok(true) => Ok(()),
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

    pub(in crate::app) fn replace_playlist_with_delivery_fence(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.replace_playlist(files, selected_index)?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn undo_playlist_change(&mut self) -> Result<(), String> {
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

    pub(in crate::app) fn undo_playlist_change_with_delivery_fence(
        &mut self,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.undo_playlist_change()?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn shuffle_remaining_playlist(&mut self) -> Result<(), String> {
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

    pub(in crate::app) fn shuffle_remaining_playlist_with_delivery_fence(
        &mut self,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.shuffle_remaining_playlist()?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn shuffle_entire_playlist(&mut self) -> Result<(), String> {
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

    pub(in crate::app) fn shuffle_entire_playlist_with_delivery_fence(
        &mut self,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        self.shuffle_entire_playlist()?;
        self.pending_playlist_protocol_delivery_fence()
    }

    pub(in crate::app) fn sync_local_playback_telemetry(
        &mut self,
        paused: Option<bool>,
        position_seconds: Option<f64>,
    ) -> Result<(), String> {
        #[cfg(test)]
        if self.take_test_failure(SessionFailurePoint::Telemetry) {
            return Err("synthetic telemetry housekeeping failure".to_owned());
        }
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PlaybackObserved {
            paused,
            position: position_seconds,
        });
        self.dispatch_application_command(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate {
                paused,
                position_seconds,
                playback_rate: None,
                paused_for_cache: None,
                cache_buffering_percent: None,
            },
        ))
        .map(|_| ())
    }

    pub(in crate::app) fn sync_local_playback_cache_state(
        &mut self,
        paused_for_cache: Option<bool>,
        cache_buffering_percent: Option<f64>,
    ) -> Result<(), String> {
        self.dispatch_application_command(ClientCommand::PlayerPlaybackObserved(
            PlayerPlaybackTelemetryUpdate {
                paused: None,
                position_seconds: None,
                playback_rate: None,
                paused_for_cache,
                cache_buffering_percent,
            },
        ))
        .map(|_| ())
    }

    pub(in crate::app) fn set_external_player_availability(
        &mut self,
        availability: ExternalPlayerAvailability,
        now_seconds: f64,
    ) -> Result<bool, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::Availability(availability));
        self.runtime
            .set_external_player_availability(availability, now_seconds)
            .map_err(|error| {
                format!("Client-core external-player availability update failed: {error}")
            })
    }

    pub(in crate::app) fn prepare_attached_playback_media(
        &mut self,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        intent: MediaLoadIntent,
        now_seconds: f64,
    ) -> Result<Option<MediaLoadPlan>, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::MediaPrepared(intent));
        let plan = if intent == MediaLoadIntent::TransportRefresh {
            self.runtime.prepare_playback_media_for_room_participation(
                logical_id,
                kind,
                now_seconds,
            )
        } else {
            self.runtime
                .prepare_playback_media_with_intent(logical_id, kind, intent, now_seconds)
        };
        Ok(Some(plan))
    }

    pub(in crate::app) fn sync_attached_player_transport_telemetry(
        &mut self,
        update: PlayerTransportTelemetryUpdate,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::TransportObserved(&update));
        Ok(gui_actions_from_playback_coordinator(
            self.runtime.observe_external_player_transport_at_epoch(
                update,
                now_seconds,
                self.playback_transport_adapter_epoch,
            ),
        ))
    }

    pub(in crate::app) fn observe_external_player_end_of_file(
        &mut self,
        completed_file: sorotte_player_api::LocalFileUpdate,
        terminal_position_seconds: Option<f64>,
        now_seconds: f64,
    ) -> Result<(), String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::EndOfFile);
        self.runtime
            .observe_external_player_end_of_file(
                completed_file,
                terminal_position_seconds,
                now_seconds,
            )
            .map_err(|error| format!("Client-core attached-player EOF observation failed: {error}"))
    }

    #[cfg(test)]
    pub(in crate::app) fn report_attached_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        accepted: bool,
        now_seconds: f64,
    ) {
        let result = if accepted {
            Ok(())
        } else {
            Err(PlayerError::OperationFailed(
                "attached player rejected coordinator command".to_owned(),
            ))
        };
        self.runtime
            .report_external_coordinator_command_dispatch(command_id, result, now_seconds);
    }

    pub(in crate::app) fn begin_attached_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        now_seconds: f64,
    ) -> Option<PlayerCommandId> {
        self.runtime
            .begin_external_coordinator_command_dispatch(command_id, now_seconds)
    }

    pub(in crate::app) fn finish_attached_coordinator_command_dispatch(
        &mut self,
        command_id: CoordinatorCommandId,
        player_command_id: Option<PlayerCommandId>,
        accepted: bool,
        now_seconds: f64,
    ) {
        let result = if accepted {
            Ok(())
        } else {
            Err(PlayerError::OperationFailed(
                "attached player rejected coordinator command".to_owned(),
            ))
        };
        self.runtime.finish_external_coordinator_command_dispatch(
            command_id,
            player_command_id,
            result,
            now_seconds,
        );
    }

    pub(in crate::app) fn playback_coordination_snapshot(
        &self,
    ) -> Option<PlaybackCoordinationSnapshot> {
        Some(self.runtime.playback_coordination_snapshot())
    }

    pub(in crate::app) fn logical_generation_for_adapter_generation(
        &self,
        adapter_generation: PlayerMediaGeneration,
    ) -> Option<u64> {
        self.runtime
            .logical_generation_for_adapter_generation(adapter_generation)
    }

    pub(in crate::app) fn keep_waiting_for_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::SeekWaitRenewed);
        Ok(gui_actions_from_playback_coordinator(
            self.runtime
                .keep_waiting_for_external_seek_preparation(now_seconds),
        ))
    }

    pub(in crate::app) fn cancel_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(gui_actions_from_playback_coordinator(
            self.runtime.cancel_external_seek_preparation(now_seconds),
        ))
    }

    pub(in crate::app) fn join_nearest_buffered_seek_preparation(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(gui_actions_from_playback_coordinator(
            self.runtime
                .join_nearest_buffered_external_seek_preparation(now_seconds),
        ))
    }

    pub(in crate::app) fn stage_attached_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PauseIntentStaged(paused));
        Ok(gui_actions_from_playback_coordinator(
            self.runtime
                .stage_external_player_pause_intent(paused, now_seconds),
        ))
    }

    pub(in crate::app) fn rollback_attached_player_pause_intent(
        &mut self,
        paused: bool,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(gui_actions_from_playback_coordinator(
            self.runtime
                .rollback_external_player_pause_intent(paused, now_seconds),
        ))
    }

    pub(in crate::app) fn take_streaming_quality_downgrade_suggestion(
        &mut self,
    ) -> Option<StreamingQualityDowngradeSuggestion> {
        let suggestion = self.runtime.streaming_quality_downgrade_suggestion(None);
        if suggestion == self.last_streaming_quality_suggestion {
            return None;
        }
        self.last_streaming_quality_suggestion = suggestion;
        suggestion
    }

    pub(in crate::app) fn take_playback_barrier_timeout_action(
        &mut self,
    ) -> Option<PlaybackBarrierTimeoutAction> {
        self.runtime.take_playback_barrier_timeout_action()
    }

    pub(in crate::app) fn reset_playback_transport_adapter_epoch(&mut self, now_seconds: f64) {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::AttachmentReset);
        self.playback_transport_adapter_epoch = self
            .runtime
            .reset_playback_transport_adapter_epoch(now_seconds);
    }

    pub(in crate::app) fn interrupt_attached_playback_recovery(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::RecoveryInterrupted);
        Ok(gui_actions_from_playback_coordinator(
            self.runtime.interrupt_external_playback_recovery(),
        ))
    }

    pub(in crate::app) fn set_playback_paused(&mut self, paused: bool) -> Result<bool, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PauseRequested(paused));
        match self.runtime.run_set_paused(paused) {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime playback pause dispatch failed: {error}"
            )),
        }
    }

    pub(in crate::app) fn emit_immediate_playback_state_update(&mut self) -> Result<bool, String> {
        #[cfg(test)]
        self.observe_for_test(SessionObservation::PlaybackStatePublication);
        let before = self.runtime.playback_coordination_snapshot();
        crate::app::test_lifecycle::record_playback_control(
            "playback-control-state-publication-before",
            crate::app::test_lifecycle::PlaybackControlObservation {
                target_paused: before.pending_local_pause_intent,
                current_room_paused: self
                    .runtime
                    .session()
                    .current_room_playstate()
                    .and_then(|playstate| playstate.paused),
                media_generation: before.media_generation,
                pending_local_pause_intent: before.pending_local_pause_intent,
                pending_local_pause_intent_dormant: before.pending_local_pause_intent_dormant,
                last_local_pause_intent_stage_accepted: before
                    .last_local_pause_intent_stage_accepted,
                transport_telemetry_observed: before.transport_telemetry_observed,
                ordinary_correction_blocked: before.ordinary_correction_blocked,
                playlist_reset_pending: self
                    .runtime
                    .session()
                    .has_pending_playlist_index_reset_intent(),
                state_queued: None,
            },
        );
        let queued = self
            .runtime
            .run_state_sync_heartbeat_with_ping(self.dont_slow_down_with_me);
        let after = self.runtime.playback_coordination_snapshot();
        crate::app::test_lifecycle::record_playback_control(
            "playback-control-state-publication-after",
            crate::app::test_lifecycle::PlaybackControlObservation {
                target_paused: after.pending_local_pause_intent,
                current_room_paused: self
                    .runtime
                    .session()
                    .current_room_playstate()
                    .and_then(|playstate| playstate.paused),
                media_generation: after.media_generation,
                pending_local_pause_intent: after.pending_local_pause_intent,
                pending_local_pause_intent_dormant: after.pending_local_pause_intent_dormant,
                last_local_pause_intent_stage_accepted: after
                    .last_local_pause_intent_stage_accepted,
                transport_telemetry_observed: after.transport_telemetry_observed,
                ordinary_correction_blocked: after.ordinary_correction_blocked,
                playlist_reset_pending: self
                    .runtime
                    .session()
                    .has_pending_playlist_index_reset_intent(),
                state_queued: Some(queued),
            },
        );
        Ok(queued)
    }

    pub(in crate::app) fn manual_seek_to_position_allowed(
        &self,
        position_seconds: f64,
    ) -> Result<bool, String> {
        Ok(self
            .runtime
            .session()
            .local_seek_target_allowed(position_seconds, system_time_seconds()))
    }

    pub(in crate::app) fn record_manual_seek_to_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<bool, String> {
        #[cfg(test)]
        if self.take_test_failure(SessionFailurePoint::SeekPublication) {
            self.observe_for_test(SessionObservation::SeekRequested {
                position: position_seconds,
                published: false,
            });
            return Ok(false);
        }
        match self.runtime.run_seek_to_position(position_seconds) {
            Ok(sent) => {
                #[cfg(test)]
                self.observe_for_test(SessionObservation::SeekRequested {
                    position: position_seconds,
                    published: sent,
                });
                Ok(sent)
            }
            Err(error) => Err(format!(
                "Client-core session runtime seek dispatch failed: {error}"
            )),
        }
    }

    pub(in crate::app) fn undo_seek(&mut self) -> Result<bool, String> {
        match self.runtime.run_undo_seek() {
            Ok(sent) => Ok(sent),
            Err(error) => Err(format!(
                "Client-core session runtime undo-seek dispatch failed: {error}"
            )),
        }
    }

    pub(in crate::app) fn pending_undo_seek_target_position(&self) -> Option<f64> {
        self.runtime
            .session()
            .last_seek_position_before_manual_seek()
    }

    pub(in crate::app) fn local_position_seconds(&self) -> Option<f64> {
        self.runtime.session().local_position_seconds()
    }

    pub(in crate::app) fn local_pause_state(&self) -> Option<bool> {
        self.runtime.session().local_paused()
    }

    pub(in crate::app) fn local_username(&self) -> Option<&str> {
        self.runtime.session().username()
    }

    pub(in crate::app) fn current_room_name(&self) -> Option<&str> {
        self.runtime.session().room()
    }

    pub(in crate::app) fn server_handshake_completed(&self) -> bool {
        self.runtime.session().is_active()
    }

    pub(in crate::app) fn current_room_playstate(&self) -> Option<GuiSessionRoomPlaystate> {
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

    pub(in crate::app) fn current_room_playstate_for_attached_player_sync(
        &self,
    ) -> Option<GuiSessionRoomPlaystate> {
        if !self
            .runtime
            .session()
            .current_room_playstate_has_remote_authority()
        {
            return None;
        }
        self.runtime
            .current_room_playstate_with_ping_now()
            .map(|playstate| GuiSessionRoomPlaystate {
                position_seconds: playstate.position,
                paused: playstate.paused,
                do_seek: playstate.do_seek,
                set_by: playstate.set_by,
            })
    }

    pub(in crate::app) fn current_room_playlist_index(&self) -> Option<usize> {
        self.projected_current_room_playlist()
            .and_then(|playlist| playlist.index)
            .and_then(|index| usize::try_from(index).ok())
    }

    pub(in crate::app) fn current_room_selected_playlist_entry(&self) -> Option<String> {
        let playlist = self.projected_current_room_playlist()?;
        let index = playlist
            .index
            .and_then(|index| usize::try_from(index).ok())?;
        playlist.files.get(index).cloned()
    }

    pub(in crate::app) fn server_media_match_supported(&self) -> bool {
        self.runtime.session().server_media_match_supported()
    }

    #[cfg(test)]
    pub(in crate::app) fn seed_playlist_reset_intent_for_test(&mut self, pause_before_sync: bool) {
        self.runtime
            .session_mut()
            .begin_local_playlist_index_reset_intent(pause_before_sync, system_time_seconds());
    }

    pub(in crate::app) fn pending_playlist_index_reset_intent(&self) -> Option<bool> {
        self.runtime.session().pending_playlist_index_reset_intent()
    }

    pub(in crate::app) fn pending_playlist_index_reset_physical_effect_applied_for_attachment(
        &self,
        player_attachment_epoch: u64,
    ) -> bool {
        self.runtime
            .session()
            .pending_playlist_index_reset_physical_effect_applied_for_attachment(
                player_attachment_epoch,
            )
    }

    pub(in crate::app) fn mark_pending_playlist_index_reset_physical_effect_applied(
        &mut self,
        player_attachment_epoch: u64,
    ) -> bool {
        self.runtime
            .session_mut()
            .mark_pending_playlist_index_reset_physical_effect_applied(player_attachment_epoch)
    }

    pub(in crate::app) fn complete_pending_playlist_index_reset_for_attachment(
        &mut self,
        player_attachment_epoch: u64,
    ) -> Option<bool> {
        self.runtime
            .session_mut()
            .complete_pending_playlist_index_reset_for_attachment(player_attachment_epoch)
    }

    pub(in crate::app) fn has_pending_playlist_index_reset_intent(&self) -> bool {
        self.runtime
            .session()
            .has_pending_playlist_index_reset_intent()
    }

    pub(in crate::app) fn pending_playlist_index_reset_has_post_selection_playstate(&self) -> bool {
        self.runtime
            .session()
            .pending_playlist_index_reset_has_post_selection_playstate()
    }

    pub(in crate::app) fn set_autoplay_enabled(&mut self, enabled: bool) -> Result<(), String> {
        let mut config = self.runtime_settings.config.clone();
        config.readiness.autoplay_initial_state = enabled;
        let active_room = self
            .runtime
            .session()
            .local_room_command_target_with_default_fallback(&self.baseline_room);
        self.dispatch_application_command(ClientCommand::update_settings(
            ClientApplicationSettings::new(config).with_active_room(active_room),
        ))?;
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

    pub(in crate::app) fn set_autoplay_threshold(
        &mut self,
        threshold: usize,
    ) -> Result<(), String> {
        let mut config = self.runtime_settings.config.clone();
        config.readiness.autoplay_min_users = AutoplayThresholdOverride::Set(threshold);
        let active_room = self
            .runtime
            .session()
            .local_room_command_target_with_default_fallback(&self.baseline_room);
        self.dispatch_application_command(ClientCommand::update_settings(
            ClientApplicationSettings::new(config).with_active_room(active_room),
        ))?;
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

    pub(in crate::app) fn set_media_match_peer_tiers(
        &mut self,
        tiers: BTreeMap<String, MediaMatchTier>,
    ) -> Result<(), String> {
        self.runtime.session_mut().set_media_match_peer_tiers(tiers);
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

    pub(in crate::app) fn current_room_media_match_peer_file_states(
        &self,
    ) -> Vec<ClientMediaMatchPeerFileState> {
        self.runtime
            .session()
            .current_room_media_match_peer_file_states()
    }

    pub(in crate::app) fn sync_runtime_settings(
        &mut self,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Result<(), String> {
        self.apply_runtime_settings_snapshot(runtime_settings)
    }

    pub(in crate::app) fn handle_local_player_unpause_attempt(
        &mut self,
    ) -> Result<GuiLocalPlayerUnpauseDecision, String> {
        if self
            .current_room_playstate_for_attached_player_sync()
            .and_then(|playstate| playstate.paused)
            != Some(true)
        {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        }

        let readiness_supported = self.runtime.session().server_readiness_supported();
        if !readiness_supported {
            return Ok(GuiLocalPlayerUnpauseDecision::NotApplicable);
        }

        let local_can_control = self.runtime.session().local_can_control().unwrap_or(false);
        if self.runtime.session().server_readiness_v2_supported() {
            // Client-core owns the exact generation-scoped V2 gate predicate.
            // Outside a Preparing readiness-owned pause, an authorized
            // controller must be able to make an ordinary Play decision.
            let gate_holds_play = self.runtime.readiness_gate_holds_current_playback();
            if gate_holds_play {
                // GUI's simple pause mirror can correct the player before its
                // later coordinator pump. Preserve an already-classified rich
                // telemetry edge here as well; core consumes it exactly once,
                // so the shared managed-loop promotion cannot duplicate it.
                self.runtime
                    .confirm_pending_native_player_play(
                        sorotte_protocol::PlayerInteractionSurface::NativePlayerControl,
                    )
                    .map_err(|error| {
                        format!("Client-core native-player readiness confirmation failed: {error}")
                    })?;
            }
            return Ok(if gate_holds_play || !local_can_control {
                GuiLocalPlayerUnpauseDecision::Block
            } else {
                GuiLocalPlayerUnpauseDecision::Allow
            });
        }

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

    pub(in crate::app) fn finalize_local_player_unpause_attempt(&mut self) -> Result<(), String> {
        #[cfg(test)]
        if self.take_test_failure(SessionFailurePoint::UnpauseFinalization) {
            return Err("synthetic readiness finalization failure".to_owned());
        }
        if self
            .current_room_playstate_for_attached_player_sync()
            .and_then(|playstate| playstate.paused)
            != Some(true)
        {
            return Ok(());
        }

        let readiness_supported = self.runtime.session().server_readiness_supported();
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

    pub(in crate::app) fn record_intentional_player_pause_action(
        &mut self,
        paused: bool,
    ) -> Result<(), String> {
        self.runtime
            .run_direct_player_readiness_intent(
                paused,
                sorotte_protocol::PlayerInteractionSurface::SorottePlaybackControl,
            )
            .map(|_| ())
            .map_err(|error| {
                format!("Client-core player readiness-intent dispatch failed: {error}")
            })
    }

    pub(in crate::app) fn begin_external_player_pause_command(
        &mut self,
        paused: bool,
        cause: PlayerCommandCause,
        now_seconds: f64,
    ) -> Result<Option<PlayerCommandId>, String> {
        Ok(self
            .runtime
            .begin_external_player_pause_command(paused, cause, now_seconds))
    }

    pub(in crate::app) fn finish_external_player_pause_command(
        &mut self,
        command_id: Option<PlayerCommandId>,
        succeeded: bool,
        now_seconds: f64,
    ) -> Result<(), String> {
        self.runtime
            .finish_external_player_pause_command(command_id, succeeded, now_seconds)
            .map_err(|error| format!("Client-core player command completion failed: {error}"))
    }

    pub(in crate::app) fn take_attached_player_local_runtime_actions(
        &mut self,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        Ok(std::mem::take(
            &mut self.pending_attached_player_local_runtime_actions,
        ))
    }

    pub(in crate::app) fn attached_player_runtime_actions(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<GuiAttachedPlayerRuntimeAction>, String> {
        let coordinator_actions = gui_actions_from_playback_coordinator(
            self.runtime.reconcile_external_player_playback(now_seconds),
        );
        let coordinator_snapshot = self.runtime.playback_coordination_snapshot();
        if coordinator_snapshot.transport_telemetry_observed
            && coordinator_snapshot.ordinary_correction_blocked
        {
            return Ok(coordinator_actions);
        }
        let Some(room_playstate) = self
            .runtime
            .current_room_playstate_with_ping_at(now_seconds)
        else {
            return Ok(Vec::new());
        };
        let Some(local_position) = self.runtime.projected_local_position_at(now_seconds) else {
            return Ok(Vec::new());
        };

        let local_can_control = self.runtime.session().local_can_control().unwrap_or(false);
        let rollback = self
            .runtime
            .session_mut()
            .desync_correction_dispatch_snapshot();
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
        let mut actions = actions
            .into_iter()
            .filter_map(|action| match action {
                ClientRuntimeAction::SetPosition(position_seconds) => {
                    Some(GuiAttachedPlayerRuntimeAction::Position(position_seconds))
                }
                ClientRuntimeAction::SetPlaybackRate(playback_rate) => {
                    Some(GuiAttachedPlayerRuntimeAction::DesyncPlaybackRate {
                        playback_rate,
                        rollback,
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        actions.splice(0..0, coordinator_actions);
        Ok(actions)
    }

    pub(in crate::app) fn restore_desync_correction_dispatch_snapshot(
        &mut self,
        snapshot: DesyncCorrectionDispatchSnapshot,
    ) -> Result<(), String> {
        self.runtime
            .session_mut()
            .restore_desync_correction_dispatch_snapshot(snapshot);
        Ok(())
    }

    pub(in crate::app) fn publish_local_file(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), String> {
        self.runtime
            .publish_local_file(file_payload, filename_privacy_mode, filesize_privacy_mode)
            .map_err(|error| {
                format!("Client-core session runtime local file publish failed: {error}")
            })
    }

    pub(in crate::app) fn connect_public_server(
        &mut self,
        selected_server: Option<(String, String)>,
    ) -> Result<(), String> {
        let Some((_label, address)) = selected_server else {
            return Err(
                "Client-core session runtime cannot connect because no public server is selected."
                    .to_owned(),
            );
        };
        let (host, _) = parse_host_and_optional_port_from_host_arg(&address);
        if host.trim().is_empty() {
            return Err(
                "Client-core session runtime cannot connect because the selected public-server address is invalid."
                    .to_owned(),
            );
        }
        self.reset_session_for_reconnect()
    }

    pub(in crate::app) fn refresh_public_servers(
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

    pub(in crate::app) fn handle_transport_disconnect(
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

    pub(in crate::app) fn drain_reconnect_delays(&mut self) -> Vec<f64> {
        self.runtime.drain_reconnect_requests()
    }

    pub(in crate::app) fn take_stop_reconnect_requested(&mut self) -> bool {
        self.runtime.take_stop_reconnect_requested()
    }

    pub(in crate::app) fn prepare_for_transport_reconnect(&mut self) -> Result<(), String> {
        self.prepare_transport_reconnect();
        Ok(())
    }

    pub(in crate::app) fn disconnect_session(&mut self, now_seconds: f64) -> Result<(), String> {
        self.runtime
            .run_disconnect(now_seconds)
            .map_err(|error| format!("Client-core session runtime disconnect failed: {error}"))
    }
}
