//! Bridge settings owns discovery, controller leases, options acknowledgements and
//! their health transitions. Inputs are requested UI settings and bridge replies;
//! outputs are leased load-script/option messages and independent health events.
//! Attachment changes retire the old lease; shutdown restores owned options and
//! releases that lease before the IPC actor stops.
use super::*;

impl MpvAdapter {
    pub(super) fn reset_legacy_syncplayintf_attachment_for_new_ipc(&mut self) {
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_last_discovery_at = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        // Health transitions are scoped to one IPC endpoint and must never outlive it.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        self.pending_chat_requests.clear();
        let connection_generation = self
            .ipc_client
            .as_ref()
            .map(MpvJsonIpcClient::generation)
            .unwrap_or_else(|| NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT.fetch_add(1, Ordering::Relaxed));
        self.legacy_syncplayintf_attachment_id = format!(
            "{}-{connection_generation}",
            self.legacy_syncplayintf_owner_id
        );
    }

    pub fn legacy_syncplay_ui_settings(&self) -> &LegacySyncplayUiSettings {
        &self.legacy_syncplay_ui_settings
    }

    pub fn last_simulated_legacy_syncplay_osd_message(
        &self,
    ) -> Option<&(String, LegacySyncplayOsdKind)> {
        self.last_simulated_legacy_syncplay_osd_message.as_ref()
    }

    pub fn legacy_syncplayintf_options_ready(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_bridge_instance_id.is_some()
            && self.legacy_syncplayintf_options_applied
            && self
                .legacy_syncplayintf_pending_options_generation
                .is_none()
    }

    pub fn legacy_syncplayintf_script_loaded(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
    }

    pub fn apply_pending_legacy_syncplayintf_options(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the syncplayintf bridge is not loaded".to_owned(),
            ));
        }
        if self.legacy_syncplayintf_options_applied {
            return Ok(());
        }
        self.send_legacy_syncplayintf_options_if_loaded()
    }

    pub fn legacy_syncplay_osd_placement_restore(&self) -> Option<(String, i64)> {
        self.legacy_syncplay_osd_placement_restore.clone()
    }

    pub fn set_legacy_syncplay_osd_placement_restore(&mut self, restore: Option<(String, i64)>) {
        self.legacy_syncplay_osd_placement_restore = restore;
    }

    pub fn load_legacy_syncplayintf_script(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), PlayerError> {
        if self.ipc_client.is_none() {
            if self.simulation_mode {
                self.legacy_syncplayintf_script_loaded = true;
                self.legacy_syncplayintf_bridge_instance_id =
                    Some("simulated-sorotte-syncplayintf".to_owned());
                self.legacy_syncplayintf_options_applied = true;
            }
            return Ok(());
        }

        if self.discover_legacy_syncplayintf_bridge(false)? {
            self.try_send_legacy_syncplayintf_options_if_pending();
            return Ok(());
        }

        let script_path = path.as_ref().to_string_lossy().into_owned();
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_LOAD_SCRIPT, script_path]))?;
        self.legacy_syncplayintf_script_name = LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned();
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        if !self.discover_legacy_syncplayintf_bridge(true)? {
            return Err(PlayerError::OperationFailed(
                "loaded the Sorotte syncplayintf resource, but its stable bridge did not answer discovery"
                    .to_owned(),
            ));
        }
        self.try_send_legacy_syncplayintf_options_if_pending();
        Ok(())
    }

    pub fn configure_legacy_syncplay_ui_settings(
        &mut self,
        settings: LegacySyncplayUiSettings,
    ) -> Result<(), PlayerError> {
        let syncplayintf_options_changed = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_differ(&settings);
        let placement_available = self.ipc_client.is_some() || self.simulation_mode;
        if placement_available && settings.should_move_osd() {
            if self.legacy_syncplay_osd_placement_restore.is_none() {
                let restore = match self.ipc_client.as_mut() {
                    Some(client) => {
                        let align = client
                            .get_property_string(MPV_PROPERTY_OSD_ALIGN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical alignment".to_owned(),
                                )
                            })?;
                        let margin = client
                            .get_property_i64(MPV_PROPERTY_OSD_MARGIN_Y)
                            .map_err(PlayerError::OperationFailed)?
                            .ok_or_else(|| {
                                PlayerError::OperationFailed(
                                    "mpv returned no current OSD vertical margin".to_owned(),
                                )
                            })?;
                        (align, margin)
                    }
                    None => ("top".to_owned(), 0),
                };
                self.legacy_syncplay_osd_placement_restore = Some(restore);
            }
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, "bottom")?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, settings.chat_osd_margin)?;
        } else if placement_available
            && let Some((align, margin)) =
                self.legacy_syncplay_osd_placement_restore.as_ref().cloned()
        {
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, &align)?;
            self.set_property_i64(MPV_PROPERTY_OSD_MARGIN_Y, margin)?;
            self.legacy_syncplay_osd_placement_restore = None;
        }
        self.legacy_syncplay_ui_settings = settings;
        if syncplayintf_options_changed {
            let runtime_bridge_was_active = matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
            );
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
            if runtime_bridge_was_active {
                self.legacy_syncplayintf_runtime_recovery_attempts = 0;
                self.legacy_syncplayintf_runtime_recovery_failure = None;
                self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::AcknowledgementTimeout,
                    "updated Chat/OSD settings are awaiting bridge acknowledgement",
                    false,
                );
                self.attempt_sorotte_bridge_runtime_recovery();
            } else {
                self.try_send_legacy_syncplayintf_options_if_pending();
            }
        }
        Ok(())
    }

    pub fn configure_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn retry_bundled_sorotte_bridge(&mut self) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.legacy_syncplayintf_runtime_rediscovery_required = false;
        self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        self.legacy_syncplayintf_runtime_recovery_failure = None;
        self.configure_bundled_sorotte_bridge_inner(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_WINDOW)
    }

    pub fn sorotte_bridge_health(&self) -> SorotteBridgeHealth {
        self.sorotte_bridge_health.clone()
    }

    /// Returns the exact settings generation acknowledged by the current bridge attachment.
    pub fn sorotte_bridge_acknowledged_generation(&self) -> Option<u64> {
        self.legacy_syncplayintf_options_applied
            .then_some(self.legacy_syncplayintf_acknowledged_options_generation)
            .flatten()
    }

    /// Advances bounded bridge maintenance and returns the oldest unconsumed health transition.
    ///
    /// Bridge transitions are independent of core mpv JSON IPC health. A `Recovering` or
    /// `Degraded` transition gates player chat and causes OSD output to use mpv's `show-text`, but
    /// does not detach the adapter or make playback commands unavailable.
    pub fn take_sorotte_bridge_health_transition(&mut self) -> Option<SorotteBridgeHealth> {
        self.maintain_runtime_integrations();
        self.pending_sorotte_bridge_health_transitions.pop_front()
    }

    /// Services only nonblocking lease/event work and returns the oldest bridge-health change.
    /// Async owners should use this variant so draining notifications cannot enter configuration
    /// retry loops or sleep while unrelated I/O futures are waiting to be polled.
    pub fn take_sorotte_bridge_health_transition_nonblocking(
        &mut self,
    ) -> Option<SorotteBridgeHealth> {
        PlayerAdapter::maintain_runtime_leases_nonblocking(self);
        self.pending_sorotte_bridge_health_transitions.pop_front()
    }

    pub fn mark_sorotte_bridge_degraded(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.degrade_sorotte_bridge(kind, reason)
    }

    pub(super) fn configure_bundled_sorotte_bridge_inner(
        &mut self,
        retry_window: Duration,
    ) -> SorotteBridgeHealth {
        let bridge_requested = self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge();
        if !bridge_requested && !self.legacy_syncplayintf_script_loaded {
            return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
        }

        if !self.legacy_syncplayintf_script_loaded {
            match self.discover_loaded_legacy_syncplayintf_script() {
                Ok(true) => {}
                Ok(false) if bridge_requested => {
                    let resource = match lease_bundled_sorotte_bridge() {
                        Ok(resource) => resource,
                        Err(error) => {
                            return self.degrade_sorotte_bridge(
                                SorotteBridgeFailureKind::ResourceMaterialization,
                                format!(
                                    "failed to materialize Sorotte's bundled mpv bridge: {error}"
                                ),
                            );
                        }
                    };
                    let script_path = match resource.load_path() {
                        Ok(path) => path,
                        Err(error) => {
                            return self.degrade_sorotte_bridge(
                                SorotteBridgeFailureKind::ResourceMaterialization,
                                format!("bundled mpv bridge changed before load-script: {error}"),
                            );
                        }
                    };
                    if let Err(error) = self.load_legacy_syncplayintf_script(&script_path) {
                        return self.degrade_sorotte_bridge(
                            SorotteBridgeFailureKind::ScriptLoad,
                            format!(
                                "failed to load Sorotte's bundled mpv bridge from '{}': {error}",
                                script_path.display()
                            ),
                        );
                    }
                }
                Ok(false) => {
                    return self.set_sorotte_bridge_health(SorotteBridgeHealth::Disabled);
                }
                Err(error) => {
                    return self.degrade_sorotte_bridge(
                        SorotteBridgeFailureKind::Discovery,
                        format!("failed to discover Sorotte's mpv bridge: {error}"),
                    );
                }
            }
        }

        let deadline = Instant::now() + retry_window;
        let mut last_acknowledged_error = None;
        let last_error = loop {
            let error = match self.apply_pending_legacy_syncplayintf_options() {
                Ok(()) if self.legacy_syncplayintf_options_ready() => {
                    let health = if bridge_requested {
                        SorotteBridgeHealth::Ready
                    } else {
                        SorotteBridgeHealth::Disabled
                    };
                    return self.set_sorotte_bridge_health(health);
                }
                Ok(()) => {
                    "Sorotte's mpv bridge did not report that its settings are ready".to_owned()
                }
                Err(error) => error.to_string(),
            };
            if let Some(acknowledged_error) = self.legacy_syncplayintf_options_ack_error.clone() {
                last_acknowledged_error = Some(acknowledged_error);
            }
            if Instant::now() >= deadline {
                break error;
            }
            std::thread::sleep(LEGACY_SYNCPLAYINTF_CONFIGURATION_RETRY_INTERVAL);
        };

        let acknowledged_error = self
            .legacy_syncplayintf_options_ack_error
            .clone()
            .or(last_acknowledged_error);
        let reason = acknowledged_error.clone().unwrap_or(last_error);
        let kind =
            classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some());
        self.degrade_sorotte_bridge(kind, reason)
    }

    pub(super) fn set_sorotte_bridge_health(
        &mut self,
        health: SorotteBridgeHealth,
    ) -> SorotteBridgeHealth {
        if self.sorotte_bridge_health == health {
            return health;
        }
        self.sorotte_bridge_health = health.clone();
        if self.pending_sorotte_bridge_health_transitions.back() != Some(&health) {
            self.pending_sorotte_bridge_health_transitions
                .push_back(health.clone());
        }
        if matches!(
            health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Disabled
        ) {
            self.legacy_syncplayintf_runtime_rediscovery_required = false;
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
            self.legacy_syncplayintf_runtime_recovery_failure = None;
        }
        health
    }

    pub(super) fn degrade_sorotte_bridge(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
    ) -> SorotteBridgeHealth {
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required =
            kind == SorotteBridgeFailureKind::LeaseBusy;
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Degraded(SorotteBridgeFailure::new(
            kind, reason,
        )))
    }

    pub(super) fn send_syncplayintf_script_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            self.legacy_syncplayintf_script_name.as_str(),
            message_name,
            payload
        ]))
    }

    pub(super) fn send_syncplayintf_probe_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<bool, PlayerError> {
        let result = match self.ipc_client.as_mut() {
            Some(client) => client.send_compatibility_probe_expect_success(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
                message_name,
                payload
            ])),
            None if self.simulation_mode => return Ok(true),
            None => return Err(PlayerError::NotConnected),
        };
        self.drain_ipc_events_if_attached();
        match result {
            Ok(()) => Ok(true),
            Err(error) if error.is_server_rejection() => Ok(false),
            Err(error) => Err(PlayerError::OperationFailed(error.into_message())),
        }
    }

    pub fn discover_loaded_legacy_syncplayintf_script(&mut self) -> Result<bool, PlayerError> {
        self.discover_legacy_syncplayintf_bridge(false)
    }

    pub(super) fn discover_legacy_syncplayintf_bridge(
        &mut self,
        wait_for_registration: bool,
    ) -> Result<bool, PlayerError> {
        if self.simulation_mode {
            self.legacy_syncplayintf_script_loaded = true;
            self.legacy_syncplayintf_bridge_instance_id =
                Some("simulated-sorotte-syncplayintf".to_owned());
            self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
            return Ok(true);
        }

        let nonce = self.legacy_syncplayintf_next_ping_nonce;
        self.legacy_syncplayintf_next_ping_nonce = self
            .legacy_syncplayintf_next_ping_nonce
            .wrapping_add(1)
            .max(1);
        self.legacy_syncplayintf_pending_ping_nonce = Some(nonce);
        let payload = json!({
            "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
            "nonce": nonce,
        })
        .to_string();
        let mut target_accepted_a_ping = false;
        let attempts = if wait_for_registration {
            LEGACY_SYNCPLAYINTF_REGISTRATION_ATTEMPTS
        } else {
            LEGACY_SYNCPLAYINTF_DISCOVERY_ATTEMPTS
        };
        for _ in 0..attempts {
            let ping_accepted =
                self.send_syncplayintf_probe_message(LEGACY_SYNCPLAYINTF_PING_MESSAGE, &payload)?;
            target_accepted_a_ping |= ping_accepted;
            if !ping_accepted {
                if !wait_for_registration {
                    self.legacy_syncplayintf_pending_ping_nonce = None;
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_some()
                && let Some(client) = self.ipc_client.as_mut()
            {
                let _ = client.get_property(MPV_PROPERTY_PAUSE);
                self.drain_ipc_events_if_attached();
            }
            if self.legacy_syncplayintf_pending_ping_nonce.is_none()
                && self.legacy_syncplayintf_bridge_instance_id.is_some()
            {
                self.legacy_syncplayintf_script_loaded = true;
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.legacy_syncplayintf_pending_ping_nonce = None;
        if target_accepted_a_ping {
            return Err(PlayerError::OperationFailed(
                "the stable Sorotte syncplayintf target accepted discovery messages but did not return a valid pong; refusing to load a duplicate bridge"
                    .to_owned(),
            ));
        }
        Ok(false)
    }

    pub(super) fn send_legacy_syncplayintf_options_if_loaded(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Err(PlayerError::OperationFailed(
                "the Sorotte syncplayintf bridge has not been discovered".to_owned(),
            ));
        }
        if self.simulation_mode {
            let generation = self.legacy_syncplayintf_next_options_generation;
            self.legacy_syncplayintf_next_options_generation = self
                .legacy_syncplayintf_next_options_generation
                .wrapping_add(1)
                .max(1);
            self.legacy_syncplayintf_options_applied = true;
            self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
            return Ok(());
        }

        let bridge_instance_id = self
            .legacy_syncplayintf_bridge_instance_id
            .clone()
            .ok_or_else(|| {
                PlayerError::OperationFailed(
                    "the Sorotte syncplayintf bridge instance is unknown".to_owned(),
                )
            })?;
        let generation = match self.legacy_syncplayintf_pending_options_generation {
            Some(generation) => generation,
            None => {
                let generation = self.legacy_syncplayintf_next_options_generation;
                self.legacy_syncplayintf_next_options_generation = self
                    .legacy_syncplayintf_next_options_generation
                    .wrapping_add(1)
                    .max(1);
                self.legacy_syncplayintf_pending_options_generation = Some(generation);
                generation
            }
        };
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_options_ack_error = None;
        let payload = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_payload(
                &bridge_instance_id,
                &self.legacy_syncplayintf_owner_id,
                &self.legacy_syncplayintf_attachment_id,
                generation,
                LEGACY_SYNCPLAYINTF_OWNER_LEASE_MS,
            );

        self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_SET_OPTIONS_MESSAGE, &payload)?;
        if !self.legacy_syncplayintf_options_applied
            && let Some(client) = self.ipc_client.as_mut()
        {
            let _ = client.get_property(MPV_PROPERTY_PAUSE);
            self.drain_ipc_events_if_attached();
        }
        if self.legacy_syncplayintf_options_applied {
            self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            return Ok(());
        }
        Err(PlayerError::OperationFailed(
            self.legacy_syncplayintf_options_ack_error
                .clone()
                .unwrap_or_else(|| {
                    format!(
                        "Sorotte syncplayintf did not acknowledge settings generation {generation}"
                    )
                }),
        ))
    }

    pub(super) fn try_send_legacy_syncplayintf_options_if_pending(&mut self) {
        if self.legacy_syncplayintf_options_applied
            || self.legacy_syncplayintf_lease_reacquire_required
            || matches!(
                self.sorotte_bridge_health,
                SorotteBridgeHealth::Recovering | SorotteBridgeHealth::Degraded(_)
            )
        {
            return;
        }

        let _ = self.send_legacy_syncplayintf_options_if_loaded();
    }

    pub(super) fn ensure_legacy_syncplayintf_ready(&mut self) -> bool {
        self.try_send_legacy_syncplayintf_options_if_pending();
        self.legacy_syncplayintf_options_ready()
    }

    pub(super) fn legacy_syncplayintf_controller_payload(&self) -> Option<String> {
        Some(
            json!({
                "protocol": LEGACY_SYNCPLAYINTF_PROTOCOL,
                "bridgeInstanceId": self.legacy_syncplayintf_bridge_instance_id.as_deref()?,
                "ownerId": self.legacy_syncplayintf_owner_id.as_str(),
                "attachmentId": self.legacy_syncplayintf_attachment_id.as_str(),
            })
            .to_string(),
        )
    }

    pub(super) fn begin_sorotte_bridge_runtime_recovery(
        &mut self,
        kind: SorotteBridgeFailureKind,
        reason: impl Into<String>,
        rediscovery_required: bool,
    ) {
        if !matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Ready | SorotteBridgeHealth::Recovering
        ) {
            return;
        }
        let reason = reason.into();
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.legacy_syncplayintf_runtime_recovery_attempts = 0;
        }
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required = true;
        self.legacy_syncplayintf_runtime_rediscovery_required |= rediscovery_required;
        self.legacy_syncplayintf_runtime_recovery_failure =
            Some(SorotteBridgeFailure::new(kind, reason));
        self.pending_chat_requests.clear();
        self.set_sorotte_bridge_health(SorotteBridgeHealth::Recovering);
    }

    pub(super) fn attempt_sorotte_bridge_runtime_recovery(&mut self) {
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering)
            || (self.legacy_syncplayintf_runtime_recovery_attempts > 0
                && self
                    .legacy_syncplayintf_last_heartbeat_at
                    .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL))
        {
            return;
        }
        self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());

        let mut forced_failure_kind = None;
        let result = if self.legacy_syncplayintf_runtime_rediscovery_required {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {
                    self.legacy_syncplayintf_runtime_rediscovery_required = false;
                    self.send_legacy_syncplayintf_options_if_loaded()
                }
                Ok(false) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(PlayerError::OperationFailed(
                        "Sorotte's stable mpv bridge target is no longer registered".to_owned(),
                    ))
                }
                Err(error) => {
                    forced_failure_kind = Some(SorotteBridgeFailureKind::Discovery);
                    Err(error)
                }
            }
        } else {
            self.send_legacy_syncplayintf_options_if_loaded()
        };

        if result.is_ok() && self.legacy_syncplayintf_options_ready() {
            let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                SorotteBridgeHealth::Ready
            } else {
                SorotteBridgeHealth::Disabled
            };
            self.set_sorotte_bridge_health(health);
            return;
        }
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            return;
        }

        self.legacy_syncplayintf_runtime_recovery_attempts += 1;
        if let Err(error) = result {
            let acknowledged_error = self.legacy_syncplayintf_options_ack_error.clone();
            let reason = acknowledged_error
                .clone()
                .unwrap_or_else(|| error.to_string());
            let kind = forced_failure_kind.unwrap_or_else(|| {
                classify_sorotte_bridge_configuration_failure(&reason, acknowledged_error.is_some())
            });
            self.legacy_syncplayintf_runtime_recovery_failure =
                Some(SorotteBridgeFailure::new(kind, reason));
        }

        if self.legacy_syncplayintf_runtime_recovery_attempts
            >= LEGACY_SYNCPLAYINTF_RUNTIME_RECOVERY_ATTEMPTS
        {
            let failure = self
                .legacy_syncplayintf_runtime_recovery_failure
                .clone()
                .unwrap_or_else(|| {
                    SorotteBridgeFailure::new(
                        SorotteBridgeFailureKind::AcknowledgementTimeout,
                        "Sorotte's mpv bridge did not acknowledge bounded runtime recovery",
                    )
                });
            self.degrade_sorotte_bridge(failure.kind, failure.reason);
        }
    }

    pub(super) fn maintain_legacy_syncplayintf_lease(&mut self) {
        self.drain_ipc_events_if_attached();
        if matches!(
            self.sorotte_bridge_health,
            SorotteBridgeHealth::Disabled | SorotteBridgeHealth::Degraded(_)
        ) {
            return;
        }
        if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }

        if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge()
            && self
                .legacy_syncplayintf_last_discovery_at
                .is_none_or(|last| last.elapsed() >= LEGACY_SYNCPLAYINTF_RUNTIME_DISCOVERY_INTERVAL)
        {
            match self.discover_legacy_syncplayintf_bridge(false) {
                Ok(true) => {}
                Ok(false) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    "Sorotte's stable mpv bridge target is no longer registered",
                    true,
                ),
                Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                    SorotteBridgeFailureKind::Discovery,
                    format!("failed to rediscover Sorotte's mpv bridge: {error}"),
                    true,
                ),
            }
            if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Recovering) {
                self.attempt_sorotte_bridge_runtime_recovery();
                return;
            }
        }

        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            self.legacy_syncplayintf_last_heartbeat_at = None;
            self.legacy_syncplayintf_pending_heartbeat_command_id = None;
            return;
        }
        if !self.legacy_syncplayintf_options_ready() {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::AcknowledgementTimeout,
                "Sorotte's mpv bridge lost its acknowledged runtime settings",
                false,
            );
            self.attempt_sorotte_bridge_runtime_recovery();
            return;
        }
        if self
            .legacy_syncplayintf_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let Some(payload) = self.legacy_syncplayintf_controller_payload() else {
            return;
        };
        match self.send_syncplayintf_script_message(LEGACY_SYNCPLAYINTF_HEARTBEAT_MESSAGE, &payload)
        {
            Ok(()) if matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Ready) => {
                self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
            }
            Ok(()) => {
                self.legacy_syncplayintf_last_heartbeat_at = None;
                self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                self.attempt_sorotte_bridge_runtime_recovery();
            }
            Err(error) => self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::IpcCommand,
                format!("failed to renew Sorotte's mpv bridge lease: {error}"),
                true,
            ),
        }
    }

    /// Queues terminal, one-way releases for Sorotte's core hook and optional bridge, then clears
    /// their local attachment state.
    ///
    /// This is a shutdown-only operation. If an IPC final write is queued, the current JSON IPC
    /// client becomes unusable; callers should invoke this immediately before detaching or
    /// replacing the adapter. Lease expiry remains the fallback when the best-effort write cannot
    /// be queued or completed.
    pub fn release_sorotte_bridge_best_effort(&mut self) {
        let mut final_commands = Vec::with_capacity(4);
        if let Some((align_y, margin_y)) = self.legacy_syncplay_osd_placement_restore.take() {
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_ALIGN_Y,
                align_y
            ]));
            final_commands.push(json!([
                MPV_COMMAND_SET_PROPERTY,
                MPV_PROPERTY_OSD_MARGIN_Y,
                margin_y
            ]));
        }
        if self
            .network_options
            .network_media_options_hook_ownership_possible
        {
            final_commands.push(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
                SOROTTE_NETWORK_OPTIONS_RELEASE_MESSAGE,
                self.network_media_options_hook_controller_payload(),
            ]));
        }
        if self.legacy_syncplayintf_script_loaded
            && let Some(payload) = self.legacy_syncplayintf_controller_payload()
        {
            final_commands.push(json!([
                MPV_COMMAND_SCRIPT_MESSAGE_TO,
                self.legacy_syncplayintf_script_name.as_str(),
                LEGACY_SYNCPLAYINTF_RELEASE_MESSAGE,
                payload
            ]));
        }
        if !final_commands.is_empty()
            && let Some(client) = self.ipc_client.as_mut()
        {
            client.send_final_commands_best_effort(final_commands);
        }
        self.reset_network_media_options_attachment_state();
        self.legacy_syncplayintf_script_loaded = false;
        self.legacy_syncplayintf_bridge_instance_id = None;
        self.legacy_syncplayintf_options_applied = false;
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = None;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_heartbeat_at = None;
        self.legacy_syncplayintf_pending_heartbeat_command_id = None;
        self.legacy_syncplayintf_lease_reacquire_required = false;
        self.pending_chat_requests.clear();
        // Release is terminal for this endpoint; queued observations are no longer actionable.
        self.pending_sorotte_bridge_health_transitions.clear();
        self.sorotte_bridge_health = SorotteBridgeHealth::Disabled;
    }

    pub(super) fn handle_legacy_syncplayintf_options_ack(&mut self, payload: Option<&str>) {
        let parsed = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok());
        let Some(parsed) = parsed else {
            if self
                .legacy_syncplayintf_pending_options_generation
                .is_some()
            {
                self.legacy_syncplayintf_options_ack_error = Some(
                    "Sorotte syncplayintf returned a malformed settings acknowledgement".to_owned(),
                );
            }
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(pending_generation) = self.legacy_syncplayintf_pending_options_generation else {
            return;
        };
        let Some(generation) = parsed.get("generation").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            self.legacy_syncplayintf_options_ack_error =
                Some("Sorotte syncplayintf acknowledgement omitted a valid generation".to_owned());
            return;
        };
        if generation < pending_generation {
            return;
        }
        if generation > pending_generation {
            self.legacy_syncplayintf_options_ack_error = Some(format!(
                "Sorotte syncplayintf acknowledged unexpected future generation {generation} while waiting for {pending_generation}"
            ));
            return;
        }
        match parsed.get("status").and_then(Value::as_str) {
            Some("applied") => {
                self.legacy_syncplayintf_options_applied = true;
                self.legacy_syncplayintf_pending_options_generation = None;
                self.legacy_syncplayintf_acknowledged_options_generation = Some(generation);
                self.legacy_syncplayintf_options_ack_error = None;
                self.legacy_syncplayintf_lease_reacquire_required = false;
                let health = if self.legacy_syncplay_ui_settings.uses_syncplayintf_bridge() {
                    SorotteBridgeHealth::Ready
                } else {
                    SorotteBridgeHealth::Disabled
                };
                self.set_sorotte_bridge_health(health);
            }
            Some(status @ ("busy" | "rejected")) => {
                let detail = parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("the bridge rejected the settings update");
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf did not apply generation {generation}: {detail}"
                ));
                if status == "busy" {
                    self.legacy_syncplayintf_lease_reacquire_required = true;
                }
            }
            _ => {
                self.legacy_syncplayintf_options_ack_error = Some(format!(
                    "Sorotte syncplayintf returned an invalid status for generation {generation}"
                ));
            }
        }
    }

    pub(super) fn handle_legacy_syncplayintf_pong(&mut self, payload: Option<&str>) {
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL) {
            return;
        }
        let Some(nonce) = parsed.get("nonce").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        if self.legacy_syncplayintf_pending_ping_nonce != Some(nonce) {
            return;
        }
        let Some(bridge_instance_id) = parsed
            .get("bridgeInstanceId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let Some(script_name) = parsed
            .get("scriptName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let bridge_instance_changed = self
            .legacy_syncplayintf_bridge_instance_id
            .as_deref()
            .is_some_and(|current| current != bridge_instance_id);
        if bridge_instance_changed {
            self.legacy_syncplayintf_options_applied = false;
            self.legacy_syncplayintf_pending_options_generation = None;
            self.legacy_syncplayintf_acknowledged_options_generation = None;
            self.legacy_syncplayintf_options_ack_error = None;
            self.legacy_syncplayintf_lease_reacquire_required = false;
        }
        self.legacy_syncplayintf_bridge_instance_id = Some(bridge_instance_id.to_owned());
        self.legacy_syncplayintf_script_name = script_name.to_owned();
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_pending_ping_nonce = None;
        self.legacy_syncplayintf_last_discovery_at = Some(Instant::now());
        if bridge_instance_changed {
            self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::Discovery,
                format!(
                    "Sorotte's mpv bridge instance changed to {bridge_instance_id}; reapplying runtime settings"
                ),
                false,
            );
        }
    }

    pub(super) fn handle_legacy_syncplayintf_lease_expired(&mut self, payload: Option<&str>) {
        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        self.legacy_syncplayintf_pending_options_generation = None;
        self.legacy_syncplayintf_acknowledged_options_generation = None;
        self.legacy_syncplayintf_options_ack_error = Some(
            "Sorotte syncplayintf input lease expired; reapplying the current settings".to_owned(),
        );
        self.begin_sorotte_bridge_runtime_recovery(
            SorotteBridgeFailureKind::AcknowledgementTimeout,
            "Sorotte syncplayintf input lease expired; reapplying the current settings",
            false,
        );
    }

    pub(super) fn handle_legacy_syncplayintf_chat_request(&mut self, payload: Option<&str>) {
        if !self.chat_input_polling_enabled() {
            return;
        }
        let Some(parsed) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        else {
            return;
        };
        if parsed.get("protocol").and_then(Value::as_str) != Some(LEGACY_SYNCPLAYINTF_PROTOCOL)
            || parsed.get("bridgeInstanceId").and_then(Value::as_str)
                != self.legacy_syncplayintf_bridge_instance_id.as_deref()
            || parsed.get("ownerId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_owner_id.as_str())
            || parsed.get("attachmentId").and_then(Value::as_str)
                != Some(self.legacy_syncplayintf_attachment_id.as_str())
        {
            return;
        }
        let Some(message) = parsed.get("text").and_then(Value::as_str) else {
            return;
        };
        self.pending_chat_requests.push_back(message.to_owned());
    }
}
