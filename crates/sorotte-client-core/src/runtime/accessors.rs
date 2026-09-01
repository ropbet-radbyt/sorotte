use super::*;
use crate::control::client_effect_player_error;
use crate::player_transition::PlayerCommandCause;
use sorotte_player_api::PlayerCommand;
use sorotte_protocol::{ControllerAuthPayload, PlaybackBarrierSetExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalControlAuthorityEvidence {
    room: Option<String>,
    username: Option<String>,
    authorized: bool,
}

pub struct ClientSessionUpdate<'a> {
    session: &'a mut ClientSession,
    control: Option<&'a mut dyn ClientEffectSink>,
    playback_coordination: Option<&'a mut RuntimePlaybackCoordination>,
}

impl<'a> ClientSessionUpdate<'a> {
    pub fn new(session: &'a mut ClientSession) -> Self {
        Self {
            session,
            control: None,
            playback_coordination: None,
        }
    }

    fn with_runtime_context(
        session: &'a mut ClientSession,
        control: &'a mut dyn ClientEffectSink,
        playback_coordination: &'a mut RuntimePlaybackCoordination,
    ) -> Self {
        Self {
            session,
            control: Some(control),
            playback_coordination: Some(playback_coordination),
        }
    }

    fn cancel_playback_barrier_request_after_room_change(&mut self, previous_room: Option<String>) {
        if previous_room.as_deref() != self.session.room() {
            if let Some(control) = self.control.as_deref_mut() {
                control.cancel_protocol_playback_barrier_requests();
                control.cancel_protocol_readiness_intents();
                control.cancel_protocol_participant_status_reports();
            }
            if let Some(playback_coordination) = self.playback_coordination.as_deref_mut() {
                playback_coordination.handle_authoritative_playback_barrier_room_change();
                playback_coordination.bind_authoritative_room_control_context(self.session);
            }
        }
    }

    fn playback_barrier_extension(
        message: &ProtocolMessage,
    ) -> Option<PlaybackBarrierSetExtension> {
        let ProtocolMessage::Set(set) = message else {
            return None;
        };
        set.set.playback_barrier_v1().ok().flatten()
    }

    fn local_control_authority_evidence(
        message: &ProtocolMessage,
        local_username: Option<&str>,
    ) -> Option<LocalControlAuthorityEvidence> {
        let ProtocolMessage::Set(set) = message else {
            return None;
        };
        if let Some(ControllerAuthPayload {
            room,
            user,
            success: Some(authorized),
            ..
        }) = set.set.controller_auth.as_ref()
        {
            return Some(LocalControlAuthorityEvidence {
                room: room.clone(),
                username: user.clone(),
                authorized: *authorized,
            });
        }
        let local_username = local_username?;
        let user = set.set.user.as_ref()?.get(local_username)?;
        Some(LocalControlAuthorityEvidence {
            room: user.room.as_ref().map(|room| room.name.clone()),
            username: Some(local_username.to_owned()),
            authorized: user.controller?,
        })
    }

    fn observe_playback_barrier_extension(
        &mut self,
        extension: Option<PlaybackBarrierSetExtension>,
        now_seconds: f64,
    ) {
        let retry_scheduled = self
            .playback_coordination
            .as_deref_mut()
            .zip(extension.as_ref())
            .is_some_and(|(playback_coordination, extension)| {
                playback_coordination.observe_playback_barrier_server_extension(
                    extension,
                    self.session,
                    now_seconds,
                )
            });
        if retry_scheduled && let Some(control) = self.control.as_deref_mut() {
            control.cancel_protocol_playback_barrier_requests();
        }
    }

    fn observe_local_control_authority(&mut self, evidence: Option<LocalControlAuthorityEvidence>) {
        let Some(evidence) = evidence else {
            return;
        };
        if let Some(playback_coordination) = self.playback_coordination.as_deref_mut() {
            playback_coordination.observe_local_control_authority(
                self.session,
                evidence.room.as_deref(),
                evidence.username.as_deref(),
                evidence.authorized,
            );
        }
    }

    fn flush_pending_readiness_reconciliation(&mut self) {
        let Some(ClientRuntimeAction::SetReadinessIntent { request, scope }) =
            self.session.pending_readiness_reconciliation_action()
        else {
            return;
        };
        let Some(control) = self.control.as_deref_mut() else {
            self.session.mark_pending_readiness_delivery_failed();
            return;
        };
        control.activate_protocol_connection_generation();
        if control
            .emit(ClientEffect::SendReadinessIntent { request, scope })
            .is_err()
        {
            self.session.mark_pending_readiness_delivery_failed();
        }
    }

    fn flush_participant_status_transition(&mut self, now_seconds: f64) {
        // An active session is established only by Hello, which always owns a
        // room. The pending-report boundary still checks that invariant
        // defensively; this gate only decides whether queued advisory reports
        // must be cancelled on capability/lifecycle withdrawal.
        let status_reporting_enabled =
            self.session.is_active() && self.session.server_participant_status_v1_supported();
        let pending = self
            .playback_coordination
            .as_deref_mut()
            .and_then(|coordination| {
                coordination.pending_participant_status_report(self.session, false, now_seconds)
            });
        if !status_reporting_enabled {
            if let Some(control) = self.control.as_deref_mut() {
                control.cancel_protocol_participant_status_reports();
            }
            return;
        }
        let Some(pending) = pending else {
            return;
        };
        let Some(control) = self.control.as_deref_mut() else {
            return;
        };
        control.activate_protocol_connection_generation();
        if control
            .emit(ClientEffect::SendState(
                StatePayload::new().with_participant_status_v1(
                    ParticipantStatusStateExtension::new().with_report(pending.report.clone()),
                ),
            ))
            .is_ok()
            && let Some(coordination) = self.playback_coordination.as_deref_mut()
        {
            coordination.commit_participant_status_report(&pending);
        }
    }

    fn cancel_participant_status_for_inactive_phase(&mut self) {
        let had_status_epoch =
            self.session.is_active() && self.session.server_participant_status_v1_supported();
        if let Some(control) = self.control.as_deref_mut() {
            // A status report belongs to the active authenticated connection.
            // Strip unleased reports immediately and mark a leased front so a
            // failed transport write cannot retry it after the lifecycle
            // boundary.
            control.cancel_protocol_participant_status_reports();
        }
        if had_status_epoch {
            emit_client_lifecycle_transition(
                "STATUS-WITHDRAW-001",
                "participant-status",
                TargetKind::ProtocolMessage,
                Trigger::Shutdown,
                Disposition::Applied,
                &[],
            );
            emit_client_lifecycle_transition(
                "STATUS-UNAVAILABLE-001",
                "participant-status",
                TargetKind::ProtocolMessage,
                Trigger::Shutdown,
                Disposition::Applied,
                &[],
            );
        }
    }

    pub fn apply_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) -> bool {
        self.session.apply_player_playback_telemetry_update(update)
    }

    pub fn initialize_local_identity(&mut self, username: String, room: String) {
        let previous_room = self.session.room().map(str::to_owned);
        self.session.initialize_local_identity(username, room);
        self.cancel_playback_barrier_request_after_room_change(previous_room);
    }

    pub fn apply_protocol_message(
        &mut self,
        message: ProtocolMessage,
    ) -> Result<(), ProtocolError> {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let is_hello = matches!(&message, ProtocolMessage::Hello(_));
        let confirms_room_membership = matches!(
            &message,
            ProtocolMessage::List(message) if matches!(&message.list, ListPayload::Rooms(_))
        );
        let extension = Self::playback_barrier_extension(&message);
        let authority_evidence =
            Self::local_control_authority_evidence(&message, self.session.username());
        let previous_room = self.session.room().map(str::to_owned);
        let result = self.session.apply_protocol_message(message);
        self.cancel_playback_barrier_request_after_room_change(previous_room);
        if result.is_ok() {
            if is_hello && let Some(control) = self.control.as_deref_mut() {
                control.activate_protocol_connection_generation();
            }
            if is_hello {
                emit_client_lifecycle_transition(
                    "SESSION-ACTIVE-001",
                    "session",
                    TargetKind::ProtocolMessage,
                    Trigger::RemoteEvent,
                    Disposition::Applied,
                    &[],
                );
                if self.session.server_participant_status_v1_supported() {
                    emit_client_lifecycle_transition(
                        "STATUS-NEGOTIATE-001",
                        "participant-status",
                        TargetKind::ProtocolMessage,
                        Trigger::RemoteEvent,
                        Disposition::Accepted,
                        &[],
                    );
                }
            }
            self.observe_playback_barrier_extension(extension, now_seconds);
            self.observe_local_control_authority(authority_evidence);
            if confirms_room_membership
                && let Some(coordination) = self.playback_coordination.as_deref_mut()
            {
                coordination.confirm_participant_status_room_membership(self.session);
            }
            self.flush_pending_readiness_reconciliation();
            self.flush_participant_status_transition(now_seconds);
        }
        result
    }

    pub fn apply_protocol_message_at(
        &mut self,
        message: ProtocolMessage,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        let is_hello = matches!(&message, ProtocolMessage::Hello(_));
        let confirms_room_membership = matches!(
            &message,
            ProtocolMessage::List(message) if matches!(&message.list, ListPayload::Rooms(_))
        );
        let extension = Self::playback_barrier_extension(&message);
        let authority_evidence =
            Self::local_control_authority_evidence(&message, self.session.username());
        let previous_room = self.session.room().map(str::to_owned);
        let result = self.session.apply_protocol_message_at(message, now_seconds);
        self.cancel_playback_barrier_request_after_room_change(previous_room);
        if result.is_ok() {
            if is_hello && let Some(control) = self.control.as_deref_mut() {
                control.activate_protocol_connection_generation();
            }
            if is_hello {
                emit_client_lifecycle_transition(
                    "SESSION-ACTIVE-001",
                    "session",
                    TargetKind::ProtocolMessage,
                    Trigger::RemoteEvent,
                    Disposition::Applied,
                    &[],
                );
                if self.session.server_participant_status_v1_supported() {
                    emit_client_lifecycle_transition(
                        "STATUS-NEGOTIATE-001",
                        "participant-status",
                        TargetKind::ProtocolMessage,
                        Trigger::RemoteEvent,
                        Disposition::Accepted,
                        &[],
                    );
                }
            }
            self.observe_playback_barrier_extension(extension, now_seconds);
            self.observe_local_control_authority(authority_evidence);
            if confirms_room_membership
                && let Some(coordination) = self.playback_coordination.as_deref_mut()
            {
                coordination.confirm_participant_status_room_membership(self.session);
            }
            self.flush_pending_readiness_reconciliation();
            self.flush_participant_status_transition(now_seconds);
        }
        result
    }

    pub fn apply_message_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        for item in decode_message_line_items(json_line)? {
            self.apply_protocol_message(item.message?)?;
        }
        Ok(())
    }

    pub fn apply_message_json_at(
        &mut self,
        json_line: &str,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        for item in decode_message_line_items(json_line)? {
            self.apply_protocol_message_at(item.message?, now_seconds)?;
        }
        Ok(())
    }

    pub fn mark_connecting(&mut self) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.mark_connecting();
        emit_client_lifecycle_transition(
            "SESSION-CONNECT-001",
            "session",
            TargetKind::ProtocolMessage,
            Trigger::Startup,
            Disposition::Submitted,
            &[],
        );
    }

    pub fn mark_awaiting_hello(&mut self) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.mark_awaiting_hello();
        emit_client_lifecycle_transition(
            "SESSION-HELLO-001",
            "session",
            TargetKind::ProtocolMessage,
            Trigger::RemoteEvent,
            Disposition::Accepted,
            &[],
        );
    }

    pub fn mark_reconnecting(&mut self, attempt: u32) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.mark_reconnecting(attempt);
        emit_client_lifecycle_transition(
            "SESSION-LOSS-001",
            "session",
            TargetKind::ProtocolMessage,
            Trigger::Fault,
            Disposition::Failed,
            &[("reconnect-attempt", u64::from(attempt).saturating_add(1))],
        );
    }

    pub fn mark_closing(&mut self) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.mark_closing();
        emit_client_lifecycle_transition(
            "SESSION-CLOSE-001",
            "session",
            TargetKind::ProtocolMessage,
            Trigger::Shutdown,
            Disposition::Accepted,
            &[],
        );
    }

    pub fn mark_disconnected(&mut self) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.mark_disconnected();
        emit_client_lifecycle_transition(
            "SESSION-DISCONNECT-001",
            "session",
            TargetKind::ProtocolMessage,
            Trigger::Shutdown,
            Disposition::Applied,
            &[],
        );
    }

    pub fn reset_sync_state_for_reconnect(&mut self) {
        self.cancel_participant_status_for_inactive_phase();
        self.session.reset_sync_state_for_reconnect();
    }

    pub fn set_reconnect_policy(&mut self, policy: ReconnectPolicyConfig) {
        self.session.set_reconnect_policy(policy);
    }

    pub fn set_behavior_config(&mut self, config: SessionBehaviorConfig) {
        self.session.set_behavior_config(config);
    }

    pub fn set_desync_config(&mut self, config: DesyncCorrectionConfig) {
        self.session.set_desync_config(config);
    }

    pub fn set_readiness_autoplay_config(&mut self, config: ReadinessAutoplayConfig) {
        self.session.set_readiness_autoplay_config(config);
    }

    pub fn set_chat_config(&mut self, config: ChatConfig) {
        self.session.set_chat_config(config);
    }

    pub fn begin_local_playlist_index_reset_intent(
        &mut self,
        pause_before_sync: bool,
        now_seconds: f64,
    ) {
        self.session
            .begin_local_playlist_index_reset_intent(pause_before_sync, now_seconds);
    }

    pub fn take_pending_playlist_index_reset_intent(&mut self) -> Option<bool> {
        self.session.take_pending_playlist_index_reset_intent()
    }

    pub fn mark_pending_playlist_index_reset_physical_effect_applied(
        &mut self,
        player_attachment_epoch: u64,
    ) -> bool {
        self.session
            .mark_pending_playlist_index_reset_physical_effect_applied(player_attachment_epoch)
    }

    pub fn complete_pending_playlist_index_reset_for_attachment(
        &mut self,
        player_attachment_epoch: u64,
    ) -> Option<bool> {
        self.session
            .complete_pending_playlist_index_reset_for_attachment(player_attachment_epoch)
    }

    pub fn runtime_actions_for_desync_correction_against_room_playstate(
        &mut self,
        room_playstate: RoomPlaystateView,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.session
            .runtime_actions_for_desync_correction_against_room_playstate(
                room_playstate,
                now_seconds,
                local_position,
                local_can_control,
                dont_slow_down_with_me,
                speed_supported,
            )
    }

    pub fn desync_correction_dispatch_snapshot(&self) -> DesyncCorrectionDispatchSnapshot {
        self.session.desync_correction_dispatch_snapshot()
    }

    pub fn restore_desync_correction_dispatch_snapshot(
        &mut self,
        snapshot: DesyncCorrectionDispatchSnapshot,
    ) {
        self.session
            .restore_desync_correction_dispatch_snapshot(snapshot);
    }

    pub fn set_autoplay_enabled(&mut self, enabled: bool) {
        self.session.set_autoplay_enabled(enabled);
    }

    pub fn set_media_match_peer_tiers(&mut self, tiers: BTreeMap<String, MediaMatchTier>) {
        self.session.set_media_match_peer_tiers(tiers);
    }

    pub fn remember_control_password_for_room(&mut self, room_name: &str, password: SecretValue) {
        self.session
            .remember_control_password_for_room(room_name, password);
    }
}

pub struct ClientPlayerIo<'a, P, C> {
    player: &'a mut P,
    playback_coordination: &'a mut RuntimePlaybackCoordination,
    session: &'a ClientSession,
    control: &'a mut C,
}

impl<P, C> ClientPlayerIo<'_, P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        let kind = if path.contains("://") {
            MediaTransportKind::NetworkVod
        } else {
            MediaTransportKind::LocalFile
        };
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or_default()
        };
        let logical_id = logical_media_id_for_local_file_update(
            &LocalFileUpdate::new(name)
                .with_size_bytes(size_bytes)
                .with_path(path),
        );
        self.open_media(
            path,
            logical_id,
            kind,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
        .map(|_| ())
    }

    pub fn unload(&mut self) -> Result<(), PlayerError> {
        self.player.unload()
    }

    pub fn open_media(
        &mut self,
        path: &str,
        logical_id: LogicalMediaId,
        kind: MediaTransportKind,
        now_seconds: f64,
    ) -> Result<MediaLoadPlan, PlayerError> {
        let cleanup_actions = self.playback_coordination.interrupt_recovery();
        for action in cleanup_actions {
            if let PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } = action
            {
                // Best effort: opening the requested source remains
                // authoritative even if the outgoing transport is gone.
                match self.player.execute(PlayerCommand::SetPlaybackRate(rate)) {
                    Ok(()) => self
                        .playback_coordination
                        .command_dispatch_succeeded(command_id),
                    Err(_) => {
                        let now_seconds = self.playback_coordination.coordinator_now(now_seconds);
                        self.playback_coordination
                            .command_dispatch_failed(command_id, now_seconds);
                    }
                }
            }
        }
        let command = PlayerCommand::OpenFile(path.to_owned());
        match self.player.execute_tracked(command.clone()) {
            Ok(_) => {}
            Err(PlayerError::Unsupported("execute_tracked")) => self.player.execute(command)?,
            Err(error) => return Err(error),
        }
        let plan = self.playback_coordination.prepare_media_with_intent(
            logical_id,
            kind,
            MediaLoadIntent::NewPlayback,
            now_seconds,
        );
        if let Some(room) = self.session.room() {
            self.control
                .retain_protocol_playback_barrier_scope(room, plan.media_generation);
        } else {
            self.control.cancel_protocol_playback_barrier_requests();
        }
        if let Some(request) = self
            .playback_coordination
            .playback_barrier_set_for_new_media(&plan, self.session, now_seconds)
        {
            self.control.activate_protocol_connection_generation();
            let scope = PlaybackBarrierRequestScope::new(
                request.room.clone(),
                request.local_media_generation,
                request.request_nonce,
            );
            self.control
                .emit(ClientEffect::send_playback_barrier_set(
                    request.extension.clone(),
                    scope,
                ))
                .map_err(client_effect_player_error)?;
            self.playback_coordination
                .confirm_playback_barrier_request_queued(&request);
        }
        if let Some(pending) = self
            .playback_coordination
            .pending_participant_status_report(self.session, false, now_seconds)
        {
            self.control.activate_protocol_connection_generation();
            if self
                .control
                .emit(ClientEffect::SendState(
                    StatePayload::new().with_participant_status_v1(
                        ParticipantStatusStateExtension::new().with_report(pending.report.clone()),
                    ),
                ))
                .is_ok()
            {
                self.playback_coordination
                    .commit_participant_status_report(&pending);
            }
        }
        Ok(plan)
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        let command = PlayerCommand::SetPaused(paused);
        let issued_at_seconds = self
            .playback_coordination
            .standalone_command_issued_at_seconds(unix_wall_clock_time_seconds_legacy_compatible());
        match self.player.execute_tracked(command.clone()) {
            Ok(command_id) => {
                self.playback_coordination
                    .bind_standalone_player_pause_command(
                        command_id,
                        PlayerCommandCause::TransportRefresh,
                        paused,
                        issued_at_seconds,
                    );
                Ok(())
            }
            Err(PlayerError::Unsupported("execute_tracked")) => {
                match self.player.execute(command) {
                    Ok(()) => {
                        self.playback_coordination
                            .register_completed_synthetic_pause_command(
                                PlayerCommandCause::TransportRefresh,
                                paused,
                                issued_at_seconds,
                            );
                        Ok(())
                    }
                    Err(error) => {
                        self.playback_coordination
                            .register_failed_synthetic_pause_command(
                                PlayerCommandCause::TransportRefresh,
                                paused,
                                issued_at_seconds,
                            );
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.playback_coordination
                    .register_failed_synthetic_pause_command(
                        PlayerCommandCause::TransportRefresh,
                        paused,
                        issued_at_seconds,
                    );
                Err(error)
            }
        }
    }

    pub fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
        self.player
            .execute(PlayerCommand::SetPosition(position_seconds))
    }

    pub fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.player.execute(PlayerCommand::SetPlaybackRate(rate))
    }
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub fn new(session: ClientSession, player: P, control: C) -> Self {
        let mut playback_coordination = RuntimePlaybackCoordination::default();
        if player
            .capabilities()
            .contains(sorotte_player_api::PlayerCapability::Telemetry)
        {
            playback_coordination.mark_transport_telemetry_available();
        }
        Self {
            session,
            player,
            control,
            ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible::default(),
            pending_player_playback_telemetry_updates: EffectOutbox::default(),
            pending_ordered_local_file_updates: EffectOutbox::default(),
            last_local_file_update: None,
            pending_natural_playback_completion: None,
            pending_reconnect_rate_reset: false,
            pending_state_sync_player_error: None,
            playback_coordination,
            ordered_player_events: OrderedPlayerEventConsumer::default(),
        }
    }

    pub(crate) fn finalize_local_playlist_selection_switch_if_needed(
        &mut self,
        selection_changed: bool,
    ) -> Result<(), PlayerError> {
        if !selection_changed {
            return Ok(());
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.session
            .begin_local_playlist_index_reset_intent(true, now_seconds);
        self.session.apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(0.0),
        );
        if !self.session.is_active() {
            return Ok(());
        }
        let playstate = self.session.with_current_transport_revision(
            PlaystatePayload::new()
                .with_position(0.0)
                .with_paused(true)
                .with_do_seek(true),
        );
        self.control.activate_protocol_connection_generation();
        self.control
            .emit_causal_state(StatePayload::new().with_playstate(playstate))
            .map_err(client_effect_player_error)
    }

    pub(crate) fn dispatch_local_seek_with_session_rollback(
        &mut self,
        session_snapshot: ClientSessionLocalActionSnapshot,
        actions: &[ClientRuntimeAction],
        causal_state: Option<StatePayload>,
    ) -> Result<(), PlayerError> {
        let result = self
            .dispatch_runtime_actions_with_causal_tracking(actions)
            .and_then(|()| {
                let Some(state) = causal_state else {
                    return Ok(());
                };
                self.control.activate_protocol_connection_generation();
                self.control
                    .emit_causal_state(state)
                    .map_err(client_effect_player_error)
            });
        match result {
            Ok(()) => Ok(()),
            Err(err) => {
                self.session.restore_local_action_state(session_snapshot);
                Err(err)
            }
        }
    }

    pub(crate) fn dispatch_runtime_actions_with_session_rollback_and_pause_cause(
        &mut self,
        session_snapshot: ClientSessionLocalActionSnapshot,
        actions: &[ClientRuntimeAction],
        pause_cause: PlayerCommandCause,
    ) -> Result<(), PlayerError> {
        match self.dispatch_runtime_actions_with_pause_cause(actions, pause_cause) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.session.restore_local_action_state(session_snapshot);
                Err(err)
            }
        }
    }

    pub(crate) fn dispatch_runtime_actions_with_causal_tracking(
        &mut self,
        actions: &[ClientRuntimeAction],
    ) -> Result<(), PlayerError> {
        for action in actions {
            if let ClientRuntimeAction::SetPaused(paused) = action {
                let cause = self.system_pause_command_cause(*paused);
                self.execute_causal_pause_command(
                    *paused,
                    cause,
                    unix_wall_clock_time_seconds_legacy_compatible(),
                )?;
            } else {
                ClientSession::dispatch_runtime_actions(
                    std::slice::from_ref(action),
                    &mut self.player,
                    &mut self.control,
                )?;
                if let ClientRuntimeAction::SetRoom { room } = action {
                    self.playback_coordination
                        .begin_participant_status_room_switch(room, self.session.room());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn dispatch_runtime_actions_with_pause_cause(
        &mut self,
        actions: &[ClientRuntimeAction],
        pause_cause: PlayerCommandCause,
    ) -> Result<(), PlayerError> {
        self.dispatch_runtime_actions_with_pause_cause_and_playlist_guard(
            actions,
            pause_cause,
            None,
        )
    }

    pub(crate) fn dispatch_runtime_actions_with_pause_cause_and_playlist_guard(
        &mut self,
        actions: &[ClientRuntimeAction],
        pause_cause: PlayerCommandCause,
        expected_playlist_state: Option<(i64, u64)>,
    ) -> Result<(), PlayerError> {
        for action in actions {
            if let ClientRuntimeAction::SetPaused(paused) = action {
                self.execute_causal_pause_command(
                    *paused,
                    pause_cause,
                    unix_wall_clock_time_seconds_legacy_compatible(),
                )?;
            } else if let (
                ClientRuntimeAction::SetPlaylistIndex { index },
                Some((expected_index, expected_epoch)),
            ) = (action, expected_playlist_state)
            {
                self.control
                    .emit_playlist_index_if_current(*index, expected_index, expected_epoch)
                    .map_err(client_effect_player_error)?;
            } else {
                ClientSession::dispatch_runtime_actions(
                    std::slice::from_ref(action),
                    &mut self.player,
                    &mut self.control,
                )?;
                if let ClientRuntimeAction::SetRoom { room } = action {
                    self.playback_coordination
                        .begin_participant_status_room_switch(room, self.session.room());
                }
            }
        }
        Ok(())
    }

    fn system_pause_command_cause(&self, paused: bool) -> PlayerCommandCause {
        if self
            .playback_coordination
            .snapshot()
            .recovery_episode
            .is_some()
        {
            return PlayerCommandCause::Recovery;
        }
        match self.session.current_room_playstate_authority() {
            Some(RoomPlaystateAuthority::ServerBarrier { .. }) if paused => {
                PlayerCommandCause::ReadinessGateHold
            }
            Some(RoomPlaystateAuthority::ServerBarrier { .. }) => {
                PlayerCommandCause::AutomaticReadinessStart
            }
            Some(RoomPlaystateAuthority::ServerBufferingPolicy { .. }) => {
                PlayerCommandCause::RoomBufferingPolicy
            }
            Some(
                RoomPlaystateAuthority::LegacyRemoteUser | RoomPlaystateAuthority::LegacyLocalEcho,
            )
            | None => PlayerCommandCause::RemoteRoomSynchronization,
        }
    }

    pub(crate) fn run_model_event(&mut self, event: ClientEvent) -> Result<(), PlayerError> {
        let mut effects = std::collections::VecDeque::from(self.session.model.apply(event));
        let mut first_error = None;
        while let Some(effect) = effects.pop_front() {
            let result = self.execute_client_effect(effect.clone());
            let feedback = match result {
                Ok(()) => ClientEvent::EffectSucceeded(effect),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    ClientEvent::EffectFailed(effect)
                }
            };
            effects.extend(self.session.model.apply(feedback));
        }
        first_error.map_or(Ok(()), Err)
    }

    fn execute_client_effect(&mut self, effect: ClientEffect) -> Result<(), PlayerError> {
        match effect {
            ClientEffect::SetPlayerPaused(paused) => {
                let v2_play_gate_hold = paused
                    && self.session.model.local_pause_change_in_flight()
                    && self.session.server_readiness_v2_supported()
                    && self
                        .session
                        .pending_readiness_intent()
                        .is_some_and(|pending| {
                            pending.desired() == sorotte_protocol::UserReadinessIntent::Ready
                        });
                let cause = if v2_play_gate_hold {
                    PlayerCommandCause::ReadinessGateHold
                } else if self.session.model.local_pause_change_in_flight() {
                    PlayerCommandCause::LocalUserPlaybackControl
                } else {
                    self.system_pause_command_cause(paused)
                };
                let local_user_transport = cause == PlayerCommandCause::LocalUserPlaybackControl;
                if local_user_transport {
                    // A local application command has the same command/echo
                    // race as a native player gesture. Stage it before player
                    // dispatch so an inbound canonical frame cannot erase the
                    // observed transport change before its State response is
                    // built. The intent is already scoped to the active room,
                    // media, connection generation, and controller authority.
                    self.playback_coordination
                        .stage_local_pause_intent(paused, &self.session);
                }
                let result = self.execute_causal_pause_command(
                    paused,
                    cause,
                    unix_wall_clock_time_seconds_legacy_compatible(),
                );
                if local_user_transport && result.is_err() {
                    self.playback_coordination
                        .rollback_local_pause_intent(paused);
                }
                result
            }
            ClientEffect::SetPlayerPosition(position) => {
                self.player.execute(PlayerCommand::SetPosition(position))
            }
            ClientEffect::SetPlayerPlaybackRate(rate) => {
                self.player.execute(PlayerCommand::SetPlaybackRate(rate))
            }
            control_effect => {
                let room_switch_target = match &control_effect {
                    ClientEffect::SetRoom(room) => Some(room.clone()),
                    _ => None,
                };
                self.control
                    .emit(control_effect)
                    .map_err(client_effect_player_error)?;
                if let Some(room) = room_switch_target {
                    self.playback_coordination
                        .begin_participant_status_room_switch(&room, self.session.room());
                }
                Ok(())
            }
        }
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    #[cfg(test)]
    pub(crate) fn session_mut_for_test(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub fn session_mut(&mut self) -> ClientSessionUpdate<'_> {
        ClientSessionUpdate::with_runtime_context(
            &mut self.session,
            &mut self.control,
            &mut self.playback_coordination,
        )
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        self.session.reconnect_state_restore_correction_metrics()
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        self.session
            .reconnect_state_restore_correction_state_snapshot()
    }

    pub fn control(&self) -> &C {
        &self.control
    }

    pub fn emit_effect(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        let room_switch_target = match &effect {
            ClientEffect::SetRoom(room) => Some(room.clone()),
            _ => None,
        };
        let result = self.control.emit(effect);
        if result.is_ok()
            && let Some(room) = room_switch_target
        {
            self.playback_coordination
                .begin_participant_status_room_switch(&room, self.session.room());
        }
        result
    }

    pub fn player(&self) -> &P {
        &self.player
    }

    #[cfg(test)]
    pub(crate) fn player_mut_for_test(&mut self) -> &mut P {
        &mut self.player
    }

    pub fn player_mut(&mut self) -> ClientPlayerIo<'_, P, C> {
        ClientPlayerIo {
            player: &mut self.player,
            playback_coordination: &mut self.playback_coordination,
            session: &self.session,
            control: &mut self.control,
        }
    }

    pub fn with_player_io<R>(&mut self, io: impl FnOnce(&mut P) -> R) -> R {
        io(&mut self.player)
    }

    pub fn last_local_file_update(&self) -> Option<&LocalFileUpdate> {
        self.last_local_file_update.as_ref()
    }

    pub fn current_room_playstate_legacy_ping_compatible_at(
        &self,
        now_seconds: f64,
    ) -> Option<RoomPlaystateView> {
        let mut room_playstate = self.session.current_room_playstate_at(now_seconds)?;
        if room_playstate.paused == Some(false)
            && let Some(position) = room_playstate.position
        {
            let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
            if forward_delay.is_finite() && forward_delay > 0.0 {
                room_playstate.position = Some(position + forward_delay);
            }
        }
        Some(room_playstate)
    }

    pub fn current_room_playstate_legacy_ping_compatible_now(&self) -> Option<RoomPlaystateView> {
        self.current_room_playstate_legacy_ping_compatible_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn projected_local_position_at(&self, now_seconds: f64) -> Option<f64> {
        self.playback_coordination
            .projected_local_position_at(now_seconds, self.session.model.playback.local_position)
    }

    pub fn into_parts(self) -> (ClientSession, P, C) {
        (self.session, self.player, self.control)
    }
}
