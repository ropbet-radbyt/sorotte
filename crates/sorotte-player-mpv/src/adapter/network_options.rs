//! Network-option supervision owns hook instance/configuration/heartbeat identity,
//! verified application diagnostics and ordered policy/health queues. Inputs are
//! desired options, authoritative path observations and typed hook messages; outputs
//! are bounded IPC commands and independent policy/health events. Attachment reset
//! clears delivery identity; media replacement fences stale application results.
//! Lua resources remain leased until the script acknowledges configuration;
//! mpv can acknowledge load-script before its Lua worker opens the file.
use super::*;

impl MpvAdapter {
    pub(super) fn reset_network_media_options_attachment_state(&mut self) {
        self.network_options.network_media_options_hook_loaded = false;
        self.network_options
            .network_media_options_hook_configured_generation = None;
        self.network_options
            .network_media_options_hook_configuration_error = None;
        self.network_options
            .network_media_options_hook_last_heartbeat_at = None;
        self.network_options
            .network_media_options_hook_pending_heartbeat = None;
        self.network_options
            .network_media_options_hook_pending_event_poll_command_id = None;
        self.network_options
            .next_network_media_options_hook_heartbeat_nonce = 1;
        self.network_options.network_media_options_hook_instance_id = None;
        self.network_options
            .network_media_options_hook_last_accepted_load_sequence = None;
        self.network_options
            .network_media_options_hook_latest_started_load_sequence = None;
        self.network_options
            .network_media_options_expected_transition = None;
        self.network_options.network_media_options_hook_health =
            MpvNetworkOptionsHookHealth::Pending;
        self.network_options
            .network_media_options_hook_ownership_possible = false;
        self.network_options
            .network_media_options_hook_configuration_in_progress = false;
        self.network_options.network_media_options_policy_state =
            MpvNetworkMediaPolicyState::Unknown;
        self.reset_network_media_policy_diagnostics();
        self.bump_network_options_runtime_health_revision();
        self.network_options
            .pending_network_media_options_hook_active_result = None;
        self.network_options
            .deferred_network_media_options_hook_transition_result = None;
        self.network_options.network_media_options_embedded_load = None;
        self.network_options.network_media_options_apply_identity = None;
        self.network_options.network_media_options_event_batch_depth = 0;
        self.network_options
            .deferred_network_media_options_observation = None;
        self.network_options
            .pending_network_options_hook_health_transitions
            .clear();
        self.network_options
            .pending_network_media_policy_outcomes
            .clear();
    }

    /// Configures options that mpv should apply only while playing network media.
    ///
    /// The options are attached to Sorotte-issued `loadfile` commands as mpv
    /// per-file options. mpv restores the user's prior values when that media
    /// ends, so a later local file keeps its normal mpv/user cache policy.
    pub fn configure_network_media_options<I, K, V>(&mut self, options: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let options = options
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        if options != self.network_options.network_media_options {
            self.network_options.network_media_options = options;
            self.network_options.network_media_options_generation = self
                .network_options
                .network_media_options_generation
                .wrapping_add(1)
                .max(1);
            self.network_options
                .network_media_options_hook_configured_generation = None;
            self.network_options
                .network_media_options_hook_configuration_error = None;
            self.network_options
                .network_media_options_hook_last_heartbeat_at = None;
            self.network_options
                .network_media_options_hook_pending_heartbeat = None;
            self.network_options
                .network_media_options_hook_pending_event_poll_command_id = None;
            if !matches!(
                self.network_options.network_media_options_hook_health,
                MpvNetworkOptionsHookHealth::Degraded(_)
            ) {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
            }
            self.network_options
                .pending_network_media_options_hook_active_result = None;
            self.network_options
                .deferred_network_media_options_hook_transition_result = None;
            self.network_options.network_media_options_embedded_load = None;
            self.network_options.network_media_options_apply_identity = None;
            self.network_options
                .network_media_options_expected_transition = None;
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
            self.reset_network_media_policy_diagnostics();
            self.network_options
                .deferred_network_media_options_observation = None;
            // File-policy results belong to the superseded option generation. Hook-health
            // transitions describe the adapter-wide hook lease and must survive unchanged,
            // including Degraded -> Recovered -> Degraded sequences that have not yet drained.
            self.network_options
                .pending_network_media_policy_outcomes
                .clear();
        }
    }

    /// Returns the oldest unconsumed hook-health transition.
    pub fn take_network_options_hook_health_transition(
        &mut self,
    ) -> Option<MpvNetworkOptionsHookHealthTransition> {
        self.maintain_runtime_integrations();
        self.take_network_options_hook_health_transition_nonblocking()
    }

    /// Pure queue pop for async wait loops that already service leases explicitly.
    pub fn take_network_options_hook_health_transition_nonblocking(
        &mut self,
    ) -> Option<MpvNetworkOptionsHookHealthTransition> {
        self.network_options
            .pending_network_options_hook_health_transitions
            .pop_front()
            .map(|event| event.value)
    }

    /// Returns the authoritative current network-options state without consuming notifications.
    pub fn network_options_runtime_health_snapshot(
        &self,
    ) -> MpvNetworkOptionsRuntimeHealthSnapshot {
        MpvNetworkOptionsRuntimeHealthSnapshot {
            revision: self
                .network_options
                .network_media_options_runtime_health_revision,
            hook_health: self
                .network_options
                .network_media_options_hook_health
                .clone(),
            media_policy: self
                .network_options
                .network_media_options_policy_state
                .clone(),
        }
    }

    /// Returns the next production-ordered compatibility outcome across the two independent
    /// channels. New consumers should drain each typed channel and reconcile the snapshot.
    pub fn take_network_media_options_transition_outcome(
        &mut self,
    ) -> Option<MpvNetworkMediaOptionsTransitionOutcome> {
        self.maintain_runtime_integrations();
        let hook_sequence = self
            .network_options
            .pending_network_options_hook_health_transitions
            .front()
            .map(|event| event.sequence);
        let policy_sequence = self
            .network_options
            .pending_network_media_policy_outcomes
            .front()
            .map(|event| event.sequence);
        match (hook_sequence, policy_sequence) {
            (Some(hook), Some(policy)) if hook <= policy => self
                .network_options
                .pending_network_options_hook_health_transitions
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkOptionsHookHealthTransition::Recovered => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookRecovered
                    }
                    MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)
                    }
                }),
            (Some(_), Some(_)) | (None, Some(_)) => self
                .network_options
                .pending_network_media_policy_outcomes
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkMediaPolicyOutcome::NoActiveMedia => {
                        MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia
                    }
                    MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged => {
                        MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged
                    }
                    MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                        MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated
                    }
                    MpvNetworkMediaPolicyOutcome::Failed(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::Failed(error)
                    }
                }),
            (Some(_), None) => self
                .network_options
                .pending_network_options_hook_health_transitions
                .pop_front()
                .map(|event| match event.value {
                    MpvNetworkOptionsHookHealthTransition::Recovered => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookRecovered
                    }
                    MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error)
                    }
                }),
            (None, None) => None,
        }
    }

    /// Applies the configured network options to an already-active network file.
    ///
    /// mpv's `file-local-options` namespace snapshots the prior option values
    /// and restores them when the file ends. This is useful when Sorotte
    /// attaches to an existing mpv session or changes settings in place. Local
    /// files are deliberately left untouched.
    pub fn apply_network_media_options_to_active_media(&mut self) -> Result<(), PlayerError> {
        self.apply_network_media_options_to_active_media_classified()
            .map(|_| ())
    }

    /// Applies configured network options and reports whether mpv had no active media, local
    /// media that was intentionally unchanged, network media that accepted every option, or a
    /// newer authoritative path superseded the explicit attempt.
    pub fn apply_network_media_options_to_active_media_classified(
        &mut self,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        let mut result = self.apply_network_media_options_to_active_media_classified_inner();
        if result
            .as_ref()
            .is_err_and(Self::network_options_hook_ownership_failure)
        {
            // A lease can expire between the last runtime pump and an explicit settings apply.
            // The Lua hook has already released the old owner, so retry the full configure/apply
            // transaction once instead of surfacing a sticky error that only an app restart can
            // clear.
            self.network_options
                .network_media_options_hook_configuration_error = None;
            result = self.apply_network_media_options_to_active_media_classified_inner();
        }
        if let Err(error) = &result {
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Failed(
                error.to_string(),
            ));
        }
        result
    }

    pub(super) fn network_options_hook_ownership_failure(error: &PlayerError) -> bool {
        let PlayerError::OperationFailed(reason) = error else {
            return false;
        };
        Self::network_options_hook_ownership_failure_reason(reason)
    }

    pub(super) fn network_options_hook_ownership_failure_reason(reason: &str) -> bool {
        reason.contains("network-options hook lease expired")
            || reason.contains("network-options hook ownership was replaced")
            || reason.contains("network-options hook ownership was lost")
            || reason.contains("network-options hook did not acknowledge heartbeat nonce")
    }

    pub(super) fn recover_network_media_options_hook_ownership_if_needed(&mut self) {
        let should_recover = matches!(
            &self.network_options.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Degraded(reason)
                if Self::network_options_hook_ownership_failure_reason(reason)
        );
        if !should_recover || !self.network_media_options_hook_should_run() {
            return;
        }

        if let Err(error) = self.apply_network_media_options_to_active_media_classified() {
            // Do not enter a blocking retry loop on every runtime tick when another live owner
            // legitimately won the lease or mpv stopped answering. The original degradation is
            // already queued for observers; record the failed automatic attempt as the current
            // authoritative health and leave subsequent recovery to the explicit retry action.
            self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Degraded(format!(
                "automatic network-options hook ownership recovery failed: {error}"
            )));
        }
    }

    pub(super) fn apply_network_media_options_to_active_media_classified_inner(
        &mut self,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        // An explicit file-policy operation must not discard adapter-wide hook-health events or
        // authoritative path results already queued by early maintenance in this same pump.
        self.ensure_network_media_options_hook_configured()?;
        // `current_path` may describe a requested load or a prior externally
        // replaced playlist entry. An attached mpv is authoritative; the cache
        // is safe only for simulation or other no-IPC operation.
        let active_path = match self.ipc_client.as_mut() {
            Some(client) => match client.get_property_string_classified(MPV_PROPERTY_PATH) {
                Ok(path) => path,
                Err(error) if error.is_property_unavailable() => None,
                Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
            },
            None => self.current_path.clone(),
        };
        let Some(active_path) = active_path else {
            if self.network_media_options_hook_should_run()
                && (self.pending_load_request().is_some()
                    || self.pending_load_generation().is_some()
                    || matches!(
                        self.transport_phase,
                        PlayerTransportPhase::Loading | PlayerTransportPhase::Prebuffering
                    ))
            {
                self.set_network_media_policy_state(
                    MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad,
                );
                return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
            }
            self.clear_network_media_options_path_identity();
            self.reset_network_media_policy_diagnostics();
            self.record_network_media_options_policy_applied(
                MpvNetworkMediaPolicyState::NoActiveMedia,
                None,
            );
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia);
        };

        let attempt_id = self
            .begin_network_media_options_apply_attempt(self.active_media_generation, &active_path);
        if self.network_media_options_hook_should_run() {
            if !self.network_media_options_hook_is_ready() {
                return Err(PlayerError::OperationFailed(
                    "Sorotte's mpv network-options hook is not ready after configuration"
                        .to_owned(),
                ));
            }
            return self
                .apply_network_media_options_to_active_media_via_hook(&active_path, attempt_id);
        }
        if !uses_network_media_options(&active_path) {
            self.clear_network_media_options_path_identity();
            self.reset_network_media_policy_diagnostics();
            self.record_network_media_options_policy_applied(
                MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                None,
            );
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged);
        }
        // Direct file-local writes are intentionally limited to simulation and explicit test
        // fixtures. A real JSON IPC attachment must never re-enter the cross-file fallback when
        // the core hook is enabled but unavailable.
        if !self.apply_network_media_options_for_attempt(&active_path, attempt_id)? {
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
        }
        self.record_unverified_network_media_options_applied();
        self.record_network_media_options_policy_applied(
            MpvNetworkMediaPolicyState::NetworkMediaUpdated,
            None,
        );
        Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated)
    }

    pub(super) fn network_media_options_hook_should_run(&self) -> bool {
        self.network_options.network_media_options_hook_enabled
            && !self.simulation_mode
            && self.ipc_client.is_some()
    }

    pub(super) fn network_media_options_hook_is_ready(&self) -> bool {
        self.network_media_options_hook_should_run()
            && matches!(
                self.network_options.network_media_options_hook_health,
                MpvNetworkOptionsHookHealth::Ready
            )
            && self.network_options.network_media_options_hook_loaded
            && self
                .network_options
                .network_media_options_hook_configured_generation
                == Some(self.network_options.network_media_options_generation)
    }

    pub(super) fn invalidate_network_media_options_hook_delivery(&mut self) {
        if matches!(
            self.network_options.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        ) {
            self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
        }
        self.network_options.network_media_options_hook_loaded = false;
        self.network_options
            .network_media_options_hook_configured_generation = None;
        self.network_options
            .network_media_options_hook_last_heartbeat_at = None;
        self.network_options
            .network_media_options_hook_pending_heartbeat = None;
        self.network_options
            .network_media_options_hook_pending_event_poll_command_id = None;
        self.network_options
            .pending_network_media_options_hook_active_result = None;
        self.network_options
            .deferred_network_media_options_hook_transition_result = None;
        self.network_options
            .network_media_options_expected_transition = None;
    }

    pub(super) fn network_media_options_hook_controller_payload(&self) -> String {
        json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_options.network_media_options_generation,
        })
        .to_string()
    }

    pub(super) fn maintain_network_media_options_hook_lease(&mut self) {
        if !self.network_media_options_hook_is_ready() {
            return;
        }
        if let Some(pending) = self
            .network_options
            .network_media_options_hook_pending_heartbeat
        {
            if pending.sent_at.is_some_and(|sent_at| {
                sent_at.elapsed() >= NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT
            }) {
                let reason = format!(
                    "Sorotte's mpv network-options hook did not acknowledge heartbeat nonce {}",
                    pending.nonce
                );
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
                    reason,
                ));
            }
            return;
        }
        if self
            .network_options
            .network_media_options_hook_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let nonce = self
            .network_options
            .next_network_media_options_hook_heartbeat_nonce;
        self.network_options
            .next_network_media_options_hook_heartbeat_nonce = self
            .network_options
            .next_network_media_options_hook_heartbeat_nonce
            .wrapping_add(1)
            .max(1);
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_options.network_media_options_generation,
            "heartbeatNonce": nonce,
        });
        self.network_options
            .network_media_options_hook_pending_heartbeat =
            Some(PendingNetworkOptionsHookHeartbeat {
                nonce,
                command_id: None,
                // Synchronous command delivery can itself take longer than the
                // Lua acknowledgement window. Start that window only after mpv
                // has accepted the script-message, matching the nonblocking
                // maintenance path.
                sent_at: None,
            });
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_HEARTBEAT_MESSAGE,
            payload.to_string(),
        ]);
        match self.send_ipc_command_if_attached(command) {
            Ok(()) => {
                if let Some(pending) = self
                    .network_options
                    .network_media_options_hook_pending_heartbeat
                    .as_mut()
                    && pending.nonce == nonce
                {
                    pending.sent_at = Some(Instant::now());
                }
            }
            Err(error) => {
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(error);
            }
        }
    }

    pub(super) fn ensure_network_media_options_hook_configured(
        &mut self,
    ) -> Result<(), PlayerError> {
        if !self.network_media_options_hook_should_run() {
            return Ok(());
        }
        if self.network_media_options_hook_is_ready() {
            return Ok(());
        }
        if self
            .network_options
            .network_media_options_hook_configuration_in_progress
        {
            return Err(PlayerError::OperationFailed(
                "Sorotte's mpv network-options hook configuration is already in progress"
                    .to_owned(),
            ));
        }
        self.network_options
            .network_media_options_hook_configuration_in_progress = true;
        let result = self.ensure_network_media_options_hook_configured_inner();
        self.network_options
            .network_media_options_hook_configuration_in_progress = false;
        result
    }

    pub(super) fn ensure_network_media_options_hook_configured_inner(
        &mut self,
    ) -> Result<(), PlayerError> {
        // Ownership/configuration failures describe the previous transaction. Leaving one set
        // here lets a delayed lease-expired event abort a new configuration before its positive
        // acknowledgement is reduced.
        self.network_options
            .network_media_options_hook_configuration_error = None;
        let _resource_lease = if !self.network_options.network_media_options_hook_loaded {
            let resource = lease_bundled_sorotte_network_options_hook().map_err(|error| {
                PlayerError::OperationFailed(format!(
                    "failed to materialize Sorotte's mpv network-options hook: {error}"
                ))
            })?;
            let path = resource.load_path().map_err(|error| {
                PlayerError::OperationFailed(format!(
                    "failed to validate network-options hook for load-script: {error}"
                ))
            })?;
            if let Err(error) = self.send_ipc_command_if_attached(json!([
                MPV_COMMAND_LOAD_SCRIPT,
                path.to_string_lossy()
            ])) {
                self.invalidate_network_media_options_hook_delivery();
                return Err(error);
            }
            self.network_options.network_media_options_hook_loaded = true;
            Some(resource)
        } else {
            None
        };
        let generation = self.network_options.network_media_options_generation;
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": generation,
            "leaseMs": NETWORK_OPTIONS_HOOK_OWNER_LEASE_MS,
            "options": self.network_media_options_map(),
        })
        .to_string();
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_CONFIGURE_MESSAGE,
            payload
        ]);
        let deadline = Instant::now() + NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_WINDOW;
        loop {
            if let Err(error) = self.send_ipc_command_if_attached(command.clone()) {
                self.invalidate_network_media_options_hook_delivery();
                return Err(error);
            }
            self.network_options
                .network_media_options_hook_ownership_possible = true;
            if self
                .network_options
                .network_media_options_hook_configured_generation
                == Some(generation)
            {
                self.network_options
                    .network_media_options_hook_last_heartbeat_at = Some(Instant::now());
                self.network_options
                    .network_media_options_hook_pending_heartbeat = None;
                self.network_options
                    .network_media_options_hook_pending_event_poll_command_id = None;
                return Ok(());
            }
            if let Some(error) = self
                .network_options
                .network_media_options_hook_configuration_error
                .take()
            {
                return Err(PlayerError::OperationFailed(error));
            }
            if Instant::now() >= deadline {
                self.invalidate_network_media_options_hook_delivery();
                return Err(PlayerError::OperationFailed(format!(
                    "Sorotte's mpv network-options hook did not acknowledge generation {generation}"
                )));
            }
            std::thread::sleep(NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_INTERVAL);
        }
    }

    pub(super) fn apply_network_media_options_to_active_media_via_hook(
        &mut self,
        initial_path: &str,
        attempt_id: u64,
    ) -> Result<MpvActiveNetworkMediaOptionsApplyOutcome, PlayerError> {
        self.network_options
            .pending_network_media_options_hook_active_result = None;
        self.set_network_media_policy_state(MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad);
        let generation = self.network_options.network_media_options_generation;
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": generation,
            "attempt": attempt_id,
        })
        .to_string();
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_APPLY_ACTIVE_MESSAGE,
            payload
        ]);
        let deadline = Instant::now() + NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_WINDOW;
        let result = loop {
            if let Err(error) = self.send_ipc_command_if_attached(command.clone()) {
                self.invalidate_network_media_options_hook_delivery();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(error);
            }
            if let Some(result) = self
                .network_options
                .pending_network_media_options_hook_active_result
                .take()
                && result.attempt_id == attempt_id
                && result.generation == generation
            {
                break result;
            }
            if let Some(error) = self
                .network_options
                .network_media_options_hook_configuration_error
                .take()
            {
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(PlayerError::OperationFailed(error));
            }
            if Instant::now() >= deadline {
                self.invalidate_network_media_options_hook_delivery();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                return Err(PlayerError::OperationFailed(format!(
                    "Sorotte's mpv network-options hook did not report active apply attempt {attempt_id}"
                )));
            }
            std::thread::sleep(NETWORK_OPTIONS_HOOK_CONFIGURATION_RETRY_INTERVAL);
        };

        let superseded = result.source_path.as_ref().map(SecretValue::expose_secret)
            != Some(initial_path)
            || !self.network_media_options_apply_attempt_is_current(attempt_id);
        if superseded {
            // The returned status belongs to the old sampled path. Only the newer authoritative
            // network/local/idle transition may publish the final outcome.
            return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded);
        }

        self.network_options
            .network_media_options_hook_last_accepted_load_sequence = Some(
            self.network_options
                .network_media_options_hook_last_accepted_load_sequence
                .map_or(result.load_sequence, |accepted| {
                    accepted.max(result.load_sequence)
                }),
        );
        self.queue_network_media_options_hook_recovered();

        match result.status {
            NetworkOptionsHookApplyStatus::NoActiveMedia => {
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::NoActiveMedia,
                    Some(result.load_sequence),
                );
                Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia)
            }
            NetworkOptionsHookApplyStatus::LocalMediaUnchanged => {
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                    Some(result.load_sequence),
                );
                Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged)
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated
            | NetworkOptionsHookApplyStatus::PartiallyApplied
            | NetworkOptionsHookApplyStatus::Failed => {
                let application_state = self.record_network_media_option_application(
                    result.load_sequence,
                    result.status,
                    result.verification_complete,
                    result.option_results,
                    result.effective_options,
                );
                if application_state == MpvNetworkMediaPolicyApplicationState::Applied {
                    self.record_network_media_options_policy_applied(
                        MpvNetworkMediaPolicyState::NetworkMediaUpdated,
                        Some(result.load_sequence),
                    );
                    return Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated);
                }
                let error = self.network_media_option_application_error(
                    result.load_sequence,
                    result.source_kind,
                    result.stream_target_kind,
                    application_state,
                );
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Failed(
                    error.to_string(),
                ));
                Err(error)
            }
        }
    }

    pub(super) fn begin_network_media_options_apply_attempt(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        path: &str,
    ) -> u64 {
        let attempt_id = self
            .network_options
            .next_network_media_options_apply_attempt_id;
        self.network_options
            .next_network_media_options_apply_attempt_id = self
            .network_options
            .next_network_media_options_apply_attempt_id
            .wrapping_add(1)
            .max(1);
        self.network_options.network_media_options_apply_identity =
            Some(NetworkMediaOptionsApplyIdentity {
                attempt_id,
                media_generation,
                path: path.to_owned(),
            });
        attempt_id
    }

    pub(super) fn network_media_options_apply_attempt_is_current(&self, attempt_id: u64) -> bool {
        self.network_options
            .network_media_options_apply_identity
            .as_ref()
            .is_some_and(|identity| identity.attempt_id == attempt_id)
    }

    pub(super) fn apply_network_media_options_for_attempt(
        &mut self,
        path: &str,
        attempt_id: u64,
    ) -> Result<bool, PlayerError> {
        if !uses_network_media_options(path) {
            return Ok(true);
        }
        for (name, value) in self.network_options.network_media_options.clone() {
            if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                return Ok(false);
            }
            let result = self.send_ipc_command_if_attached(json!([
                MPV_COMMAND_SET_PROPERTY,
                format!("file-local-options/{name}"),
                value
            ]));
            if let Err(error) = result {
                // Command rejection returns before the generic sender drains events that arrived
                // ahead of the response. Process them before attributing the error so a newer
                // authoritative path can supersede this attempt without a stale failure outcome.
                self.drain_ipc_events_if_attached();
                // Supersession can make a healthy rejection irrelevant to the new file, but an
                // unhealthy transport is adapter-wide and must remain observable to its owner.
                if !self.is_connected() {
                    return Err(error);
                }
                if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                    return Ok(false);
                }
                return Err(error);
            }
            if !self.network_media_options_apply_attempt_is_current(attempt_id) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn embedded_network_media_options_belong_to_pending_load(&self) -> bool {
        self.network_options
            .network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| {
                self.pending_load_generation() == Some(embedded.media_generation)
                    || (self.pending_load_request().is_some()
                        && self.active_media_generation == Some(embedded.media_generation))
            })
    }

    pub(super) fn embedded_network_media_options_apply_to_path(
        &self,
        media_generation: Option<PlayerMediaGeneration>,
        path: &str,
    ) -> bool {
        self.network_options
            .network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| {
                Some(embedded.media_generation) == media_generation
                    && Self::media_target_matches(path, &embedded.requested_target)
            })
    }

    pub(super) fn clear_network_media_options_path_identity(&mut self) {
        self.network_options.network_media_options_apply_identity = None;
        if !self.embedded_network_media_options_belong_to_pending_load() {
            self.network_options.network_media_options_embedded_load = None;
        }
    }

    pub(super) fn record_network_media_options_policy_applied(
        &mut self,
        state: MpvNetworkMediaPolicyState,
        load_sequence: Option<u64>,
    ) {
        self.set_network_media_policy_state(state);
        if let Some(load_sequence) = load_sequence {
            self.network_options
                .network_media_options_hook_last_accepted_load_sequence = Some(
                self.network_options
                    .network_media_options_hook_last_accepted_load_sequence
                    .map_or(load_sequence, |accepted| accepted.max(load_sequence)),
            );
        }
    }

    pub(super) fn bump_network_options_runtime_health_revision(&mut self) {
        self.network_options
            .network_media_options_runtime_health_revision = self
            .network_options
            .network_media_options_runtime_health_revision
            .wrapping_add(1)
            .max(1);
    }

    pub(super) fn set_network_options_hook_health(&mut self, health: MpvNetworkOptionsHookHealth) {
        if self.network_options.network_media_options_hook_health != health {
            self.network_options.network_media_options_hook_health = health;
            self.bump_network_options_runtime_health_revision();
        }
    }

    pub(super) fn next_network_options_event_sequence(&mut self) -> u64 {
        let sequence = self.network_options.next_network_options_event_sequence;
        self.network_options.next_network_options_event_sequence = self
            .network_options
            .next_network_options_event_sequence
            .wrapping_add(1)
            .max(1);
        sequence
    }

    pub(super) fn queue_network_options_hook_health_transition(
        &mut self,
        transition: MpvNetworkOptionsHookHealthTransition,
    ) {
        match &transition {
            MpvNetworkOptionsHookHealthTransition::Recovered => {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
            }
            MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                if matches!(
                    self.network_options.network_media_options_hook_health,
                    MpvNetworkOptionsHookHealth::Degraded(_)
                ) {
                    return;
                }
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Degraded(
                    error.to_string(),
                ));
            }
        }
        if self
            .network_options
            .pending_network_options_hook_health_transitions
            .len()
            == MAX_PENDING_NETWORK_MEDIA_OPTIONS_TRANSITION_OUTCOMES
        {
            self.network_options
                .pending_network_options_hook_health_transitions
                .pop_front();
        }
        let sequence = self.next_network_options_event_sequence();
        self.network_options
            .pending_network_options_hook_health_transitions
            .push_back(SequencedNetworkOptionsEvent {
                sequence,
                value: transition,
            });
    }

    pub(super) fn queue_network_media_options_hook_degraded(&mut self, error: PlayerError) {
        self.queue_network_options_hook_health_transition(
            MpvNetworkOptionsHookHealthTransition::Degraded(error),
        );
    }

    pub(super) fn queue_network_media_options_hook_recovered(&mut self) {
        let was_degraded = matches!(
            self.network_options.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Degraded(_)
        );
        self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        if was_degraded {
            self.queue_network_options_hook_health_transition(
                MpvNetworkOptionsHookHealthTransition::Recovered,
            );
        }
    }

    pub(super) fn network_media_option_allows_diagnostic_value(name: &str) -> bool {
        NETWORK_MEDIA_OPTION_READBACK_ALLOWLIST.contains(&name)
    }

    pub(super) fn network_media_options_desired_cache_options(&self) -> BTreeMap<String, String> {
        self.network_options
            .network_media_options
            .iter()
            .filter_map(|(name, value)| {
                Self::canonical_network_media_diagnostic_value(name, value)
                    .map(|value| (name.clone(), value))
            })
            .collect()
    }

    pub(super) fn record_unverified_network_media_options_applied(&mut self) {
        self.network_options.network_media_options_application_state =
            Some(MpvNetworkMediaPolicyApplicationState::Applied);
        self.network_options
            .network_media_options_diagnostic_load_sequence = None;
        self.network_options
            .network_media_options_verification_complete = false;
        self.network_options.network_media_options_option_results = self
            .network_options
            .network_media_options
            .keys()
            .map(|name| MpvNetworkOptionApplyResult {
                name: name.clone(),
                status: MpvNetworkOptionApplyStatus::Applied,
            })
            .collect();
        self.network_options
            .network_media_options_effective_cache_options
            .clear();
    }

    pub(super) fn normalize_mpv_boolean(value: &str) -> Option<bool> {
        match value.trim().to_ascii_lowercase().as_str() {
            "yes" | "true" | "on" | "1" => Some(true),
            "no" | "false" | "off" | "0" => Some(false),
            _ => None,
        }
    }

    pub(super) fn parse_mpv_byte_quantity(value: &str) -> Option<f64> {
        let normalized = value.trim().to_ascii_lowercase().replace(' ', "");
        let (number, multiplier) = [
            ("gib", 1024.0 * 1024.0 * 1024.0),
            ("mib", 1024.0 * 1024.0),
            ("kib", 1024.0),
            ("gb", 1_000_000_000.0),
            ("mb", 1_000_000.0),
            ("kb", 1_000.0),
            ("b", 1.0),
        ]
        .into_iter()
        .find_map(|(suffix, multiplier)| {
            normalized
                .strip_suffix(suffix)
                .map(|number| (number, multiplier))
        })
        .unwrap_or((normalized.as_str(), 1.0));
        let bytes = number.parse::<f64>().ok()? * multiplier;
        (bytes.is_finite() && bytes >= 0.0).then_some(bytes)
    }

    pub(super) fn network_media_option_values_match(
        name: &str,
        desired: &str,
        effective: &str,
    ) -> bool {
        if desired.trim().eq_ignore_ascii_case(effective.trim()) {
            return true;
        }
        match name {
            "cache-pause" | "cache-pause-initial" | "cache-on-disk" => {
                Self::normalize_mpv_boolean(desired) == Self::normalize_mpv_boolean(effective)
                    && Self::normalize_mpv_boolean(desired).is_some()
            }
            "cache-pause-wait" | "cache-secs" => {
                let Some(desired) = desired.trim().parse::<f64>().ok() else {
                    return false;
                };
                let Some(effective) = effective.trim().parse::<f64>().ok() else {
                    return false;
                };
                desired.is_finite()
                    && effective.is_finite()
                    && (desired - effective).abs() <= 0.000_001
            }
            "demuxer-max-bytes" | "demuxer-max-back-bytes" => {
                let Some(desired) = Self::parse_mpv_byte_quantity(desired) else {
                    return false;
                };
                let Some(effective) = Self::parse_mpv_byte_quantity(effective) else {
                    return false;
                };
                (desired - effective).abs() <= 1.0
            }
            _ => false,
        }
    }

    pub(super) fn record_network_media_option_application(
        &mut self,
        load_sequence: u64,
        hook_status: NetworkOptionsHookApplyStatus,
        verification_complete: bool,
        hook_results: Vec<NetworkOptionsHookOptionResult>,
        effective_options: BTreeMap<String, String>,
    ) -> MpvNetworkMediaPolicyApplicationState {
        let mut results = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for hook_result in hook_results {
            if !self
                .network_options
                .network_media_options
                .contains_key(&hook_result.name)
                || !seen.insert(hook_result.name.clone())
            {
                continue;
            }
            let status = match hook_result.status {
                NetworkOptionsHookOptionApplyStatus::Rejected => {
                    MpvNetworkOptionApplyStatus::Rejected
                }
                NetworkOptionsHookOptionApplyStatus::Applied
                    if verification_complete
                        && Self::network_media_option_allows_diagnostic_value(
                            &hook_result.name,
                        ) =>
                {
                    match (
                        self.network_options
                            .network_media_options
                            .get(&hook_result.name),
                        effective_options.get(&hook_result.name),
                    ) {
                        (Some(desired), Some(effective))
                            if Self::network_media_option_values_match(
                                &hook_result.name,
                                desired,
                                effective,
                            ) =>
                        {
                            MpvNetworkOptionApplyStatus::Applied
                        }
                        _ => MpvNetworkOptionApplyStatus::Mismatched,
                    }
                }
                NetworkOptionsHookOptionApplyStatus::Applied => {
                    MpvNetworkOptionApplyStatus::Applied
                }
            };
            results.push(MpvNetworkOptionApplyResult {
                name: hook_result.name,
                status,
            });
        }

        if verification_complete {
            for name in self.network_options.network_media_options.keys() {
                if seen.insert(name.clone()) {
                    results.push(MpvNetworkOptionApplyResult {
                        name: name.clone(),
                        status: MpvNetworkOptionApplyStatus::Mismatched,
                    });
                }
            }
        }

        let applied = results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Applied)
            .count();
        let problematic = results.len().saturating_sub(applied);
        let state = match hook_status {
            NetworkOptionsHookApplyStatus::Failed => MpvNetworkMediaPolicyApplicationState::Failed,
            NetworkOptionsHookApplyStatus::PartiallyApplied => {
                if applied == 0 {
                    MpvNetworkMediaPolicyApplicationState::Failed
                } else {
                    MpvNetworkMediaPolicyApplicationState::PartiallyApplied
                }
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated if problematic == 0 => {
                MpvNetworkMediaPolicyApplicationState::Applied
            }
            _ if applied == 0 => MpvNetworkMediaPolicyApplicationState::Failed,
            _ => MpvNetworkMediaPolicyApplicationState::PartiallyApplied,
        };

        self.network_options.network_media_options_application_state = Some(state);
        self.network_options
            .network_media_options_diagnostic_load_sequence = Some(load_sequence);
        self.network_options
            .network_media_options_verification_complete = verification_complete;
        self.network_options.network_media_options_option_results = results;
        self.network_options
            .network_media_options_effective_cache_options = effective_options
            .into_iter()
            .filter(|(name, _)| {
                self.network_options
                    .network_media_options
                    .contains_key(name)
                    && Self::network_media_option_allows_diagnostic_value(name)
            })
            .collect();
        state
    }

    pub(super) fn network_media_option_application_error(
        &self,
        load_sequence: u64,
        source_kind: NetworkOptionsMediaTargetKind,
        stream_target_kind: NetworkOptionsMediaTargetKind,
        state: MpvNetworkMediaPolicyApplicationState,
    ) -> PlayerError {
        if state == MpvNetworkMediaPolicyApplicationState::Failed
            && self
                .network_options
                .network_media_options_option_results
                .is_empty()
        {
            return NetworkOptionsApplyDiagnostic::player_error(
                load_sequence,
                source_kind,
                stream_target_kind,
            );
        }
        let applied = self
            .network_options
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Applied)
            .count();
        let rejected = self
            .network_options
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Rejected)
            .count();
        let mismatched = self
            .network_options
            .network_media_options_option_results
            .iter()
            .filter(|result| result.status == MpvNetworkOptionApplyStatus::Mismatched)
            .count();
        let classification = match state {
            MpvNetworkMediaPolicyApplicationState::Applied => "applied",
            MpvNetworkMediaPolicyApplicationState::PartiallyApplied => "partially applied",
            MpvNetworkMediaPolicyApplicationState::Failed => "failed to apply",
        };
        PlayerError::OperationFailed(format!(
            "mpv {classification} the network-media policy for hook load {load_sequence} (source: {source_kind}, resolved target: {stream_target_kind}; {applied} applied, {rejected} rejected, {mismatched} mismatched)"
        ))
    }

    pub(super) fn network_media_options_map(&self) -> serde_json::Map<String, Value> {
        self.network_options
            .network_media_options
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect()
    }

    pub(super) fn read_authoritative_property_at_response_boundary_with_network_options_flush(
        &mut self,
        attachment_epoch: PlayerAttachmentEpoch,
        property_name: &str,
        flush_network_options: bool,
    ) -> Result<Option<Value>, ()> {
        let response = match self.ipc_client.as_mut() {
            Some(client) => client.get_property_classified(property_name),
            None => {
                self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                    attachment_epoch,
                });
                return Err(());
            }
        };

        // The worker encountered every queued event before it returned this
        // response. Reduce that causal prefix before applying the response;
        // events harvested by a later property query can then supersede it.
        if flush_network_options {
            self.drain_ipc_events_if_attached();
        } else {
            self.drain_ipc_events_without_network_options_flush();
        }

        match response {
            Ok(value) => Ok(value),
            Err(error) if error.is_property_unavailable() => Ok(None),
            Err(_) => {
                self.apply_lifecycle_input(PlayerLifecycleInput::LifecycleReconciliationFailed {
                    attachment_epoch,
                });
                self.observe_unhealthy_ipc_transport();
                Err(())
            }
        }
    }

    pub(super) fn drain_ipc_events_without_network_options_flush(&mut self) -> bool {
        self.reduce_pending_ipc_events(false)
    }

    pub(super) fn flush_deferred_network_media_options_observation(&mut self) {
        let observation = self
            .network_options
            .deferred_network_media_options_observation
            .take();
        let observed_path = observation
            .as_ref()
            .map(|observation| observation.path.clone());
        if let Some(observation) = observation {
            self.apply_authoritative_path_for_network_options(
                observation.path.as_deref(),
                observation.origin,
            );
        }
        if let Some(result) = self
            .network_options
            .deferred_network_media_options_hook_transition_result
            .take()
        {
            self.apply_network_options_hook_transition_result(result, observed_path);
        }
    }

    pub(super) fn flush_deferred_network_media_options_for_authoritative_path(
        &mut self,
        path: Option<&str>,
    ) {
        // Every deferred path observation preceded the completed path query,
        // so the query result supersedes it. The hook transition itself still
        // belongs to this start-file boundary and must be finalized only after
        // the playlist response has bound its media generation.
        self.network_options
            .deferred_network_media_options_observation = None;
        if let Some(result) = self
            .network_options
            .deferred_network_media_options_hook_transition_result
            .take()
        {
            self.apply_network_options_hook_transition_result(
                result,
                Some(path.map(ToOwned::to_owned)),
            );
        }
        self.apply_authoritative_path_for_network_options(
            path,
            AuthoritativePathObservationOrigin::Poll,
        );
    }

    pub(super) fn parse_network_options_hook_status(
        parsed: &Value,
    ) -> Option<NetworkOptionsHookApplyStatus> {
        let wire_status = parsed.get("status").and_then(Value::as_str)?;
        match (
            wire_status,
            parsed.get("applicationState").and_then(Value::as_str),
        ) {
            ("network-updated", Some("applied")) => {
                return Some(NetworkOptionsHookApplyStatus::NetworkMediaUpdated);
            }
            ("failed", Some("partially-applied")) => {
                return Some(NetworkOptionsHookApplyStatus::PartiallyApplied);
            }
            ("failed", Some("failed")) => return Some(NetworkOptionsHookApplyStatus::Failed),
            (_, Some(_)) => return None,
            _ => {}
        }
        match wire_status {
            "no-active" => Some(NetworkOptionsHookApplyStatus::NoActiveMedia),
            "local" => Some(NetworkOptionsHookApplyStatus::LocalMediaUnchanged),
            "network-updated" => Some(NetworkOptionsHookApplyStatus::NetworkMediaUpdated),
            // Accepted for compatibility with short-lived development builds. The bundled v3
            // hook uses legacy `failed` plus `applicationState=partially-applied`, so an older
            // v3 adapter still fails closed instead of silently ignoring a new wire status.
            "partially-applied" => Some(NetworkOptionsHookApplyStatus::PartiallyApplied),
            "failed" => Some(NetworkOptionsHookApplyStatus::Failed),
            _ => None,
        }
    }

    pub(super) fn parse_network_options_hook_option_results(
        parsed: &Value,
    ) -> Vec<NetworkOptionsHookOptionResult> {
        parsed
            .get("optionResults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|result| {
                let name = result
                    .get("name")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_owned();
                if name.is_empty() {
                    return None;
                }
                let status = match result.get("status").and_then(Value::as_str)? {
                    "applied" => NetworkOptionsHookOptionApplyStatus::Applied,
                    "rejected" => NetworkOptionsHookOptionApplyStatus::Rejected,
                    _ => return None,
                };
                Some(NetworkOptionsHookOptionResult { name, status })
            })
            .collect()
    }

    pub(super) fn parse_network_options_hook_effective_options(
        parsed: &Value,
    ) -> BTreeMap<String, String> {
        parsed
            .get("effectiveOptions")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| {
                Self::canonical_network_media_diagnostic_value(name, value.as_str()?)
                    .map(|value| (name.clone(), value))
            })
            .collect()
    }

    pub(super) fn network_options_hook_verification_complete(parsed: &Value) -> bool {
        parsed.get("verification").and_then(Value::as_str) == Some("complete")
    }

    pub(super) fn parse_network_options_hook_payload(
        &self,
        payload: Option<&str>,
    ) -> Option<Value> {
        let parsed = serde_json::from_str::<Value>(payload?).ok()?;
        (parsed.get("protocol").and_then(Value::as_str) == Some(SOROTTE_NETWORK_OPTIONS_PROTOCOL)
            && parsed.get("ownerId").and_then(Value::as_str)
                == Some(self.legacy_syncplayintf_owner_id.as_str())
            && parsed.get("attachmentId").and_then(Value::as_str)
                == Some(self.legacy_syncplayintf_attachment_id.as_str()))
        .then_some(parsed)
    }

    pub(super) fn network_options_hook_generation(parsed: &Value) -> Option<u64> {
        parsed.get("configurationGeneration").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    pub(super) fn network_options_hook_load_sequence(parsed: &Value) -> Option<u64> {
        parsed.get("loadSequence").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    pub(super) fn network_options_hook_current_load_sequence(parsed: &Value) -> Option<u64> {
        parsed.get("currentLoadSequence").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    }

    pub(super) fn network_options_hook_instance_id(parsed: &Value) -> Option<&str> {
        parsed
            .get("hookInstanceId")
            .and_then(Value::as_str)
            .filter(|instance_id| !instance_id.is_empty())
    }

    pub(super) fn network_options_hook_matches_configured_instance(&self, parsed: &Value) -> bool {
        matches!(
            (
                Self::network_options_hook_instance_id(parsed),
                self.network_options.network_media_options_hook_instance_id.as_deref(),
            ),
            (Some(received), Some(configured)) if received == configured
        )
    }

    pub(super) fn network_options_hook_path(parsed: &Value, key: &str) -> Option<String> {
        parsed
            .get(key)
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn handle_network_options_hook_configured(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            self.network_options
                .network_media_options_hook_configuration_error =
                Some("Sorotte's mpv network-options hook omitted a valid generation".to_owned());
            return;
        };
        if generation != self.network_options.network_media_options_generation {
            return;
        }
        match parsed.get("status").and_then(Value::as_str) {
            Some("configured") => {
                let Some(hook_instance_id) = Self::network_options_hook_instance_id(&parsed) else {
                    self.network_options
                        .network_media_options_hook_configuration_error = Some(
                        "Sorotte's mpv network-options hook omitted its instance id".to_owned(),
                    );
                    return;
                };
                let Some(current_load_sequence) =
                    Self::network_options_hook_current_load_sequence(&parsed)
                else {
                    self.network_options
                        .network_media_options_hook_configuration_error = Some(
                        "Sorotte's mpv network-options hook omitted its current load sequence"
                            .to_owned(),
                    );
                    return;
                };
                if self
                    .network_options
                    .network_media_options_hook_instance_id
                    .as_deref()
                    == Some(hook_instance_id)
                {
                    if self
                        .network_options
                        .network_media_options_hook_last_accepted_load_sequence
                        .is_some_and(|accepted| current_load_sequence < accepted)
                    {
                        let accepted = self
                            .network_options
                            .network_media_options_hook_last_accepted_load_sequence
                            .expect("the regression guard established an accepted sequence");
                        let reason = format!(
                            "Sorotte's mpv network-options hook reported a regressed load sequence ({current_load_sequence} below {accepted}) for the same instance"
                        );
                        self.invalidate_network_media_options_hook_delivery();
                        self.network_options
                            .pending_network_media_options_hook_active_result = None;
                        self.network_options
                            .deferred_network_media_options_hook_transition_result = None;
                        self.network_options
                            .network_media_options_hook_configuration_error = Some(reason.clone());
                        self.queue_network_media_options_hook_degraded(
                            PlayerError::OperationFailed(reason),
                        );
                        return;
                    }
                    self.network_options
                        .network_media_options_hook_last_accepted_load_sequence = Some(
                        self.network_options
                            .network_media_options_hook_last_accepted_load_sequence
                            .map_or(current_load_sequence, |accepted| {
                                accepted.max(current_load_sequence)
                            }),
                    );
                    self.network_options
                        .network_media_options_hook_latest_started_load_sequence = Some(
                        self.network_options
                            .network_media_options_hook_latest_started_load_sequence
                            .map_or(current_load_sequence, |started| {
                                started.max(current_load_sequence)
                            }),
                    );
                } else {
                    self.network_options.network_media_options_hook_instance_id =
                        Some(hook_instance_id.to_owned());
                    self.network_options
                        .network_media_options_hook_last_accepted_load_sequence =
                        Some(current_load_sequence);
                    self.network_options
                        .network_media_options_hook_latest_started_load_sequence =
                        Some(current_load_sequence);
                    self.network_options
                        .network_media_options_expected_transition = None;
                    self.network_options
                        .pending_network_media_options_hook_active_result = None;
                    self.network_options
                        .deferred_network_media_options_hook_transition_result = None;
                }
                self.network_options.network_media_options_hook_loaded = true;
                self.network_options
                    .network_media_options_hook_ownership_possible = true;
                self.network_options
                    .network_media_options_hook_configured_generation = Some(generation);
                self.network_options
                    .network_media_options_hook_configuration_error = None;
                self.network_options
                    .network_media_options_hook_last_heartbeat_at = Some(Instant::now());
                self.network_options
                    .network_media_options_hook_pending_heartbeat = None;
                self.network_options
                    .network_media_options_hook_pending_event_poll_command_id = None;
                self.queue_network_media_options_hook_recovered();
            }
            Some("stale") => {
                self.network_options
                    .network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook rejected stale generation {generation}"
                ));
            }
            Some("owner-live") => {
                let active_owner = parsed
                    .get("activeOwnerId")
                    .and_then(Value::as_str)
                    .unwrap_or("another Sorotte process");
                self.network_options
                    .network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook is owned by {active_owner}"
                ));
            }
            _ => {
                self.network_options
                    .network_media_options_hook_configuration_error = Some(format!(
                    "Sorotte's mpv network-options hook returned an invalid status for generation {generation}"
                ));
            }
        }
    }

    pub(super) fn handle_network_options_hook_ownership(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let status = parsed.get("status").and_then(Value::as_str);
        let reason = match status {
            Some("ownership-lost") => "Sorotte's mpv network-options hook ownership was replaced",
            Some("lease-expired") => "Sorotte's mpv network-options hook lease expired",
            Some("released") => {
                self.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Pending);
                self.network_options
                    .network_media_options_hook_ownership_possible = false;
                self.network_options
                    .network_media_options_hook_configured_generation = None;
                self.network_options
                    .network_media_options_hook_last_heartbeat_at = None;
                self.network_options
                    .network_media_options_hook_pending_heartbeat = None;
                self.network_options
                    .network_media_options_hook_pending_event_poll_command_id = None;
                self.network_options.network_media_options_hook_instance_id = None;
                self.network_options
                    .network_media_options_hook_last_accepted_load_sequence = None;
                self.network_options
                    .network_media_options_hook_latest_started_load_sequence = None;
                self.network_options
                    .network_media_options_expected_transition = None;
                self.network_options
                    .pending_network_media_options_hook_active_result = None;
                return;
            }
            _ => return,
        };
        self.network_options
            .network_media_options_hook_configured_generation = None;
        self.network_options
            .network_media_options_hook_last_heartbeat_at = None;
        self.network_options
            .network_media_options_hook_pending_heartbeat = None;
        self.network_options
            .network_media_options_hook_pending_event_poll_command_id = None;
        self.network_options.network_media_options_hook_instance_id = None;
        self.network_options
            .network_media_options_hook_last_accepted_load_sequence = None;
        self.network_options
            .network_media_options_hook_latest_started_load_sequence = None;
        self.network_options
            .network_media_options_expected_transition = None;
        self.network_options
            .network_media_options_hook_ownership_possible = false;
        self.network_options
            .pending_network_media_options_hook_active_result = None;
        self.network_options
            .network_media_options_hook_configuration_error = Some(reason.to_owned());
        self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
            reason.to_owned(),
        ));
    }

    pub(super) fn handle_network_options_hook_heartbeat(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        if Self::network_options_hook_generation(&parsed)
            != self
                .network_options
                .network_media_options_hook_configured_generation
            || parsed.get("status").and_then(Value::as_str) != Some("renewed")
        {
            return;
        }
        let Some(nonce) = parsed.get("heartbeatNonce").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        if self
            .network_options
            .network_media_options_hook_pending_heartbeat
            .is_some_and(|pending| pending.nonce == nonce)
        {
            self.network_options
                .network_media_options_hook_pending_heartbeat = None;
            self.network_options
                .network_media_options_hook_pending_event_poll_command_id = None;
            self.network_options
                .network_media_options_hook_last_heartbeat_at = Some(Instant::now());
            self.queue_network_media_options_hook_recovered();
        }
    }

    pub(super) fn handle_network_options_hook_active_result(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            return;
        };
        let Some(attempt_id) = parsed.get("attempt").and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        }) else {
            return;
        };
        let Some(status) = Self::parse_network_options_hook_status(&parsed) else {
            return;
        };
        let Some(load_sequence) = Self::network_options_hook_load_sequence(&parsed) else {
            return;
        };
        let source_path = Self::network_options_hook_path(&parsed, "sourcePath");
        let stream_open_filename = Self::network_options_hook_path(&parsed, "streamOpenFilename");
        self.network_options
            .pending_network_media_options_hook_active_result =
            Some(NetworkOptionsHookActiveResult {
                attempt_id,
                generation,
                load_sequence,
                source_kind: NetworkOptionsMediaTargetKind::from_target(source_path.as_deref()),
                stream_target_kind: NetworkOptionsMediaTargetKind::from_target(
                    stream_open_filename.as_deref(),
                ),
                source_path: source_path.map(SecretValue::from),
                status,
                verification_complete: Self::network_options_hook_verification_complete(&parsed),
                option_results: Self::parse_network_options_hook_option_results(&parsed),
                effective_options: Self::parse_network_options_hook_effective_options(&parsed),
            });
    }

    pub(super) fn handle_network_options_hook_transition_result(&mut self, payload: Option<&str>) {
        let Some(parsed) = self.parse_network_options_hook_payload(payload) else {
            return;
        };
        if !self.network_options_hook_matches_configured_instance(&parsed) {
            return;
        }
        let Some(generation) = Self::network_options_hook_generation(&parsed) else {
            return;
        };
        if Some(generation)
            != self
                .network_options
                .network_media_options_hook_configured_generation
        {
            return;
        }
        let Some(status) = Self::parse_network_options_hook_status(&parsed) else {
            return;
        };
        let Some(load_sequence) = Self::network_options_hook_load_sequence(&parsed) else {
            return;
        };
        if self
            .network_options
            .network_media_options_expected_transition
            .is_some_and(|expected| {
                self.active_media_generation != Some(expected.media_generation)
                    || load_sequence < expected.load_sequence
            })
        {
            return;
        }
        if self
            .network_options
            .network_media_options_hook_last_accepted_load_sequence
            .is_some_and(|accepted| load_sequence <= accepted)
            || self
                .network_options
                .deferred_network_media_options_hook_transition_result
                .as_ref()
                .is_some_and(|pending| load_sequence <= pending.load_sequence)
        {
            return;
        }
        let source_path = Self::network_options_hook_path(&parsed, "sourcePath");
        let stream_open_filename = Self::network_options_hook_path(&parsed, "streamOpenFilename");
        self.network_options
            .deferred_network_media_options_hook_transition_result =
            Some(NetworkOptionsHookTransitionResult {
                generation,
                load_sequence,
                source_kind: NetworkOptionsMediaTargetKind::from_target(source_path.as_deref()),
                stream_target_kind: NetworkOptionsMediaTargetKind::from_target(
                    stream_open_filename.as_deref(),
                ),
                source_path: source_path.map(SecretValue::from),
                status,
                verification_complete: Self::network_options_hook_verification_complete(&parsed),
                option_results: Self::parse_network_options_hook_option_results(&parsed),
                effective_options: Self::parse_network_options_hook_effective_options(&parsed),
            });
    }

    pub(super) fn apply_network_options_hook_transition_result(
        &mut self,
        result: NetworkOptionsHookTransitionResult,
        observed_path: Option<Option<String>>,
    ) {
        if result.generation != self.network_options.network_media_options_generation
            || self
                .network_options
                .network_media_options_hook_last_accepted_load_sequence
                .is_some_and(|accepted| result.load_sequence <= accepted)
        {
            return;
        }
        if let Some(expected) = self
            .network_options
            .network_media_options_expected_transition
        {
            if self.active_media_generation != Some(expected.media_generation)
                || result.load_sequence < expected.load_sequence
            {
                return;
            }
            if let Some(Some(observed_path)) = observed_path.as_ref()
                && result.source_path.as_ref().is_none_or(|source| {
                    !Self::media_target_matches(source.expose_secret(), observed_path)
                })
            {
                return;
            }
            self.network_options
                .network_media_options_expected_transition = None;
        }
        self.network_options
            .network_media_options_hook_latest_started_load_sequence = Some(
            self.network_options
                .network_media_options_hook_latest_started_load_sequence
                .map_or(result.load_sequence, |started| {
                    started.max(result.load_sequence)
                }),
        );
        self.network_options
            .network_media_options_hook_last_accepted_load_sequence = Some(result.load_sequence);
        self.queue_network_media_options_hook_recovered();

        let completes_pending_policy = self
            .network_options
            .network_media_options_apply_identity
            .is_some()
            || matches!(
                self.network_options.network_media_options_policy_state,
                MpvNetworkMediaPolicyState::Failed(_)
                    | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
            );
        match result.status {
            NetworkOptionsHookApplyStatus::NoActiveMedia => {
                self.clear_network_media_options_path_identity();
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::NoActiveMedia,
                    Some(result.load_sequence),
                );
                if completes_pending_policy {
                    self.queue_network_media_policy_outcome(
                        MpvNetworkMediaPolicyOutcome::NoActiveMedia,
                    );
                }
            }
            NetworkOptionsHookApplyStatus::LocalMediaUnchanged => {
                if let Some(path) = result.source_path.as_ref().map(SecretValue::expose_secret) {
                    self.begin_network_media_options_apply_attempt(
                        self.active_media_generation,
                        path,
                    );
                }
                self.reset_network_media_policy_diagnostics();
                self.record_network_media_options_policy_applied(
                    MpvNetworkMediaPolicyState::LocalMediaUnchanged,
                    Some(result.load_sequence),
                );
                self.queue_network_media_policy_outcome(
                    MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged,
                );
            }
            NetworkOptionsHookApplyStatus::NetworkMediaUpdated
            | NetworkOptionsHookApplyStatus::PartiallyApplied
            | NetworkOptionsHookApplyStatus::Failed => {
                if let Some(path) = result.source_path.as_ref().map(SecretValue::expose_secret) {
                    self.begin_network_media_options_apply_attempt(
                        self.active_media_generation,
                        path,
                    );
                }
                let application_state = self.record_network_media_option_application(
                    result.load_sequence,
                    result.status,
                    result.verification_complete,
                    result.option_results,
                    result.effective_options,
                );
                if application_state == MpvNetworkMediaPolicyApplicationState::Applied {
                    self.record_network_media_options_policy_applied(
                        MpvNetworkMediaPolicyState::NetworkMediaUpdated,
                        Some(result.load_sequence),
                    );
                    self.queue_network_media_policy_outcome(
                        MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated,
                    );
                } else {
                    let error = self.network_media_option_application_error(
                        result.load_sequence,
                        result.source_kind,
                        result.stream_target_kind,
                        application_state,
                    );
                    self.queue_network_media_policy_outcome(MpvNetworkMediaPolicyOutcome::Failed(
                        error,
                    ));
                }
            }
        }
    }
}

pub(super) struct SequencedNetworkOptionsEvent<T> {
    pub(super) sequence: u64,
    pub(super) value: T,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct NetworkMediaOptionsApplyIdentity {
    pub(super) attempt_id: u64,
    pub(super) media_generation: Option<PlayerMediaGeneration>,
    pub(super) path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ExpectedNetworkOptionsTransition {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) load_sequence: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct EmbeddedNetworkMediaOptions {
    pub(super) media_generation: PlayerMediaGeneration,
    pub(super) requested_target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NetworkOptionsHookApplyStatus {
    NoActiveMedia,
    LocalMediaUnchanged,
    NetworkMediaUpdated,
    PartiallyApplied,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NetworkOptionsHookOptionApplyStatus {
    Applied,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetworkOptionsHookOptionResult {
    pub(super) name: String,
    pub(super) status: NetworkOptionsHookOptionApplyStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetworkOptionsHookActiveResult {
    pub(super) attempt_id: u64,
    pub(super) generation: u64,
    pub(super) load_sequence: u64,
    pub(super) source_path: Option<SecretValue>,
    pub(super) source_kind: NetworkOptionsMediaTargetKind,
    pub(super) stream_target_kind: NetworkOptionsMediaTargetKind,
    pub(super) status: NetworkOptionsHookApplyStatus,
    pub(super) verification_complete: bool,
    pub(super) option_results: Vec<NetworkOptionsHookOptionResult>,
    pub(super) effective_options: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetworkOptionsHookTransitionResult {
    pub(super) generation: u64,
    pub(super) load_sequence: u64,
    pub(super) source_path: Option<SecretValue>,
    pub(super) source_kind: NetworkOptionsMediaTargetKind,
    pub(super) stream_target_kind: NetworkOptionsMediaTargetKind,
    pub(super) status: NetworkOptionsHookApplyStatus,
    pub(super) verification_complete: bool,
    pub(super) option_results: Vec<NetworkOptionsHookOptionResult>,
    pub(super) effective_options: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NetworkOptionsMediaTargetKind {
    Absent,
    LocalPath,
    FileUrl,
    Http,
    Https,
    Edl,
    OtherProtocol,
}

impl NetworkOptionsMediaTargetKind {
    pub(super) fn from_target(target: Option<&str>) -> Self {
        let Some(target) = target.map(str::trim).filter(|target| !target.is_empty()) else {
            return Self::Absent;
        };
        let Some((scheme, _)) = target.split_once("://") else {
            return Self::LocalPath;
        };
        match scheme.to_ascii_lowercase().as_str() {
            "file" => Self::FileUrl,
            "http" => Self::Http,
            "https" => Self::Https,
            "edl" => Self::Edl,
            _ => Self::OtherProtocol,
        }
    }
}

impl fmt::Display for NetworkOptionsMediaTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "none",
            Self::LocalPath => "local path",
            Self::FileUrl => "file URL",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Edl => "EDL",
            Self::OtherProtocol => "other protocol",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NetworkOptionsApplyDiagnostic {
    pub(super) load_sequence: u64,
    pub(super) source_kind: NetworkOptionsMediaTargetKind,
    pub(super) stream_target_kind: NetworkOptionsMediaTargetKind,
}

impl NetworkOptionsApplyDiagnostic {
    pub(super) fn player_error(
        load_sequence: u64,
        source_kind: NetworkOptionsMediaTargetKind,
        stream_target_kind: NetworkOptionsMediaTargetKind,
    ) -> PlayerError {
        PlayerError::OperationFailed(
            Self {
                load_sequence,
                source_kind,
                stream_target_kind,
            }
            .to_string(),
        )
    }
}

impl fmt::Display for NetworkOptionsApplyDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "mpv rejected a network-media option for hook load {} (source: {}, resolved target: {})",
            self.load_sequence, self.source_kind, self.stream_target_kind
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingNetworkOptionsHookHeartbeat {
    pub(super) nonce: u64,
    /// Present only for heartbeats sent through the asynchronous control lane. Synchronous
    /// maintenance observes delivery directly and therefore does not need completion identity.
    pub(super) command_id: Option<u64>,
    /// Set only after mpv has accepted the heartbeat command. A nonblocking IPC command can
    /// remain in flight longer than the hook acknowledgement window, so starting that window at
    /// enqueue time would falsely degrade an otherwise healthy hook.
    pub(super) sent_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthoritativePathObservationOrigin {
    StartFilePending,
    PathEvent,
    Poll,
    EndFileIdle,
}

pub(super) struct DeferredAuthoritativePathObservation {
    pub(super) path: Option<String>,
    pub(super) origin: AuthoritativePathObservationOrigin,
}

pub(super) struct NetworkOptionsState {
    pub(super) network_media_options: BTreeMap<String, String>,
    pub(super) network_media_options_hook_enabled: bool,
    pub(super) network_media_options_hook_loaded: bool,
    pub(super) network_media_options_generation: u64,
    pub(super) network_media_options_hook_configured_generation: Option<u64>,
    pub(super) network_media_options_hook_configuration_error: Option<String>,
    pub(super) network_media_options_hook_last_heartbeat_at: Option<Instant>,
    pub(super) network_media_options_hook_pending_heartbeat:
        Option<PendingNetworkOptionsHookHeartbeat>,
    pub(super) network_media_options_hook_pending_event_poll_command_id: Option<u64>,
    pub(super) next_network_media_options_hook_heartbeat_nonce: u64,
    pub(super) network_media_options_hook_instance_id: Option<String>,
    pub(super) network_media_options_hook_last_accepted_load_sequence: Option<u64>,
    pub(super) network_media_options_hook_latest_started_load_sequence: Option<u64>,
    pub(super) network_media_options_expected_transition: Option<ExpectedNetworkOptionsTransition>,
    pub(super) network_media_options_hook_health: MpvNetworkOptionsHookHealth,
    pub(super) network_media_options_hook_ownership_possible: bool,
    pub(super) network_media_options_hook_configuration_in_progress: bool,
    pub(super) network_media_options_policy_state: MpvNetworkMediaPolicyState,
    pub(super) network_media_options_runtime_health_revision: u64,
    pub(super) network_media_options_application_state:
        Option<MpvNetworkMediaPolicyApplicationState>,
    pub(super) network_media_options_diagnostic_load_sequence: Option<u64>,
    pub(super) network_media_options_verification_complete: bool,
    pub(super) network_media_options_option_results: Vec<MpvNetworkOptionApplyResult>,
    pub(super) network_media_options_effective_cache_options: BTreeMap<String, String>,
    pub(super) pending_network_media_options_hook_active_result:
        Option<NetworkOptionsHookActiveResult>,
    pub(super) deferred_network_media_options_hook_transition_result:
        Option<NetworkOptionsHookTransitionResult>,
    pub(super) network_media_options_embedded_load: Option<EmbeddedNetworkMediaOptions>,
    pub(super) network_media_options_apply_identity: Option<NetworkMediaOptionsApplyIdentity>,
    pub(super) next_network_media_options_apply_attempt_id: u64,
    pub(super) network_media_options_event_batch_depth: usize,
    pub(super) deferred_network_media_options_observation:
        Option<DeferredAuthoritativePathObservation>,
    pub(super) next_network_options_event_sequence: u64,
    pub(super) pending_network_options_hook_health_transitions:
        VecDeque<SequencedNetworkOptionsEvent<MpvNetworkOptionsHookHealthTransition>>,
    pub(super) pending_network_media_policy_outcomes:
        VecDeque<SequencedNetworkOptionsEvent<MpvNetworkMediaPolicyOutcome>>,
}

impl Default for NetworkOptionsState {
    fn default() -> Self {
        Self {
            network_media_options: BTreeMap::new(),
            network_media_options_hook_enabled: true,
            network_media_options_hook_loaded: false,
            network_media_options_generation: 1,
            network_media_options_hook_configured_generation: None,
            network_media_options_hook_configuration_error: None,
            network_media_options_hook_last_heartbeat_at: None,
            network_media_options_hook_pending_heartbeat: None,
            network_media_options_hook_pending_event_poll_command_id: None,
            next_network_media_options_hook_heartbeat_nonce: 1,
            network_media_options_hook_instance_id: None,
            network_media_options_hook_last_accepted_load_sequence: None,
            network_media_options_hook_latest_started_load_sequence: None,
            network_media_options_expected_transition: None,
            network_media_options_hook_health: MpvNetworkOptionsHookHealth::Pending,
            network_media_options_hook_ownership_possible: false,
            network_media_options_hook_configuration_in_progress: false,
            network_media_options_policy_state: MpvNetworkMediaPolicyState::Unknown,
            network_media_options_runtime_health_revision: 0,
            network_media_options_application_state: None,
            network_media_options_diagnostic_load_sequence: None,
            network_media_options_verification_complete: false,
            network_media_options_option_results: Vec::new(),
            network_media_options_effective_cache_options: BTreeMap::new(),
            pending_network_media_options_hook_active_result: None,
            deferred_network_media_options_hook_transition_result: None,
            network_media_options_embedded_load: None,
            network_media_options_apply_identity: None,
            next_network_media_options_apply_attempt_id: 1,
            network_media_options_event_batch_depth: 0,
            deferred_network_media_options_observation: None,
            next_network_options_event_sequence: 1,
            pending_network_options_hook_health_transitions: VecDeque::new(),
            pending_network_media_policy_outcomes: VecDeque::new(),
        }
    }
}

impl MpvAdapter {
    /// Returns the oldest unconsumed active-media policy outcome.
    pub fn take_network_media_policy_outcome(&mut self) -> Option<MpvNetworkMediaPolicyOutcome> {
        self.maintain_runtime_integrations();
        self.take_network_media_policy_outcome_nonblocking()
    }

    /// Pure queue pop for async wait loops that already service leases explicitly.
    pub fn take_network_media_policy_outcome_nonblocking(
        &mut self,
    ) -> Option<MpvNetworkMediaPolicyOutcome> {
        self.network_options
            .pending_network_media_policy_outcomes
            .pop_front()
            .map(|event| event.value)
    }

    /// Returns generation-correlated effective policy and cache state without retaining media
    /// targets or arbitrary advanced option values.
    pub fn network_media_diagnostic_snapshot(&self) -> MpvNetworkMediaDiagnosticSnapshot {
        MpvNetworkMediaDiagnosticSnapshot {
            media_generation: self.observation_media_generation(),
            network_policy_generation: self.network_options.network_media_options_generation,
            load_sequence: self
                .network_options
                .network_media_options_diagnostic_load_sequence,
            application_state: self.network_options.network_media_options_application_state,
            verification_complete: self
                .network_options
                .network_media_options_verification_complete,
            option_results: self
                .network_options
                .network_media_options_option_results
                .clone(),
            desired_cache_options: self.network_media_options_desired_cache_options(),
            effective_cache_options: self
                .network_options
                .network_media_options_effective_cache_options
                .clone(),
            observed_at: self.observed_state.cache_metrics_observed_at,
            transport_phase: self.transport_phase,
            paused_for_cache: self.observed_state.paused_for_cache,
            demuxer_cache_idle: self.observed_state.demuxer_cache_idle,
            cache_duration_seconds: self.observed_state.buffered_ahead_seconds,
            forward_bytes: self.observed_state.buffered_ahead_bytes,
            raw_input_rate_bytes_per_second: self.observed_state.input_rate_bytes_per_second,
            reader_position_seconds: self.observed_state.cache_reader_position_seconds,
            cache_end_seconds: self.observed_state.cache_end_seconds,
            cache_eof: self.observed_state.cache_eof,
            cache_underrun: self.observed_state.cache_underrun,
        }
    }

    pub(super) fn observe_authoritative_path_for_network_options(
        &mut self,
        path: Option<&str>,
        origin: AuthoritativePathObservationOrigin,
    ) {
        if self.network_options.network_media_options_event_batch_depth != 0 {
            if origin == AuthoritativePathObservationOrigin::StartFilePending {
                self.network_options
                    .deferred_network_media_options_hook_transition_result = None;
            }

            if path.is_none() {
                if origin == AuthoritativePathObservationOrigin::EndFileIdle
                    && self
                        .network_options
                        .deferred_network_media_options_observation
                        .as_ref()
                        .is_some_and(|observation| {
                            observation.origin == AuthoritativePathObservationOrigin::Poll
                                && observation.path.is_none()
                        })
                {
                    self.network_options
                        .deferred_network_media_options_observation =
                        Some(DeferredAuthoritativePathObservation {
                            path: None,
                            origin: AuthoritativePathObservationOrigin::EndFileIdle,
                        });
                    return;
                }
                if origin != AuthoritativePathObservationOrigin::StartFilePending
                    && self
                        .network_options
                        .deferred_network_media_options_observation
                        .as_ref()
                        .is_some_and(|observation| {
                            observation.origin == AuthoritativePathObservationOrigin::EndFileIdle
                                && observation.path.is_none()
                        })
                {
                    return;
                }
            }
            // A poll issued while reducing an already-buffered event batch completes after every
            // event already present in that batch. Preserve that newer snapshot over events that
            // are merely handled later from the older local batch vector.
            if self
                .network_options
                .deferred_network_media_options_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.origin == AuthoritativePathObservationOrigin::Poll
                        && origin != AuthoritativePathObservationOrigin::Poll
                })
            {
                return;
            }
            self.network_options
                .deferred_network_media_options_observation =
                Some(DeferredAuthoritativePathObservation {
                    path: path.map(ToOwned::to_owned),
                    origin,
                });
            return;
        }
        self.apply_authoritative_path_for_network_options(path, origin);
    }

    pub(super) fn apply_authoritative_path_for_network_options(
        &mut self,
        path: Option<&str>,
        origin: AuthoritativePathObservationOrigin,
    ) {
        let Some(path) = path else {
            let completes_pending_policy = self
                .network_options
                .network_media_options_apply_identity
                .is_some()
                || matches!(
                    self.network_options.network_media_options_policy_state,
                    MpvNetworkMediaPolicyState::Failed(_)
                        | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad
                );
            self.clear_network_media_options_path_identity();
            if origin == AuthoritativePathObservationOrigin::EndFileIdle {
                self.reset_network_media_policy_diagnostics();
                self.set_network_media_policy_state(MpvNetworkMediaPolicyState::NoActiveMedia);
            }
            if self.network_media_options_hook_should_run()
                && origin == AuthoritativePathObservationOrigin::EndFileIdle
                && completes_pending_policy
                && self
                    .network_options
                    .deferred_network_media_options_hook_transition_result
                    .is_none()
            {
                self.queue_network_media_policy_outcome(
                    MpvNetworkMediaPolicyOutcome::NoActiveMedia,
                );
            }
            return;
        };
        let media_generation = self.active_media_generation;
        if self.network_options.network_media_options.is_empty() {
            return;
        }

        if self.network_media_options_hook_should_run() {
            let duplicate = self
                .network_options
                .network_media_options_apply_identity
                .as_ref()
                .is_some_and(|identity| {
                    identity.path == path
                        && (identity.media_generation == media_generation
                            || identity.media_generation.is_none())
                });
            if duplicate {
                return;
            }

            let recovered_after_on_load = !self.network_media_options_hook_is_ready();
            if recovered_after_on_load
                && self
                    .network_options
                    .network_media_options_hook_configuration_in_progress
            {
                return;
            }
            if recovered_after_on_load
                && let Err(error) = self.ensure_network_media_options_hook_configured()
            {
                self.queue_network_media_options_hook_degraded(error);
                return;
            }
            let attempt_id = self.begin_network_media_options_apply_attempt(media_generation, path);
            if self.embedded_network_media_options_apply_to_path(media_generation, path) {
                self.network_options.network_media_options_embedded_load = None;
            }
            if recovered_after_on_load {
                let outcome = match self
                    .apply_network_media_options_to_active_media_via_hook(path, attempt_id)
                {
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::Superseded) => return,
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NoActiveMedia) => {
                        MpvNetworkMediaPolicyOutcome::NoActiveMedia
                    }
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::LocalMediaUnchanged) => {
                        MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged
                    }
                    Ok(MpvActiveNetworkMediaOptionsApplyOutcome::NetworkMediaUpdated) => {
                        MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated
                    }
                    Err(error) if !self.network_media_options_hook_is_ready() => {
                        self.queue_network_media_options_hook_degraded(error);
                        return;
                    }
                    Err(error) => MpvNetworkMediaPolicyOutcome::Failed(error),
                };
                self.queue_network_media_policy_outcome(outcome);
            }
            return;
        }

        if !uses_network_media_options(path) {
            self.network_options.network_media_options_apply_identity = None;
            self.reset_network_media_policy_diagnostics();
            self.set_network_media_policy_state(MpvNetworkMediaPolicyState::LocalMediaUnchanged);
            let embedded_generation_is_current = self
                .network_options
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| Some(embedded.media_generation) == media_generation);
            if embedded_generation_is_current
                && !self.embedded_network_media_options_belong_to_pending_load()
            {
                self.network_options.network_media_options_embedded_load = None;
            }
            return;
        }

        let duplicate = self
            .network_options
            .network_media_options_apply_identity
            .as_ref()
            .is_some_and(|identity| {
                identity.path == path
                    && (identity.media_generation == media_generation
                        || identity.media_generation.is_none())
            });
        if duplicate {
            if let Some(identity) = self
                .network_options
                .network_media_options_apply_identity
                .as_mut()
                && identity.media_generation.is_none()
            {
                identity.media_generation = media_generation;
            }
            return;
        }

        if self.embedded_network_media_options_apply_to_path(media_generation, path) {
            self.begin_network_media_options_apply_attempt(media_generation, path);
            self.network_options.network_media_options_embedded_load = None;
            self.record_unverified_network_media_options_applied();
            self.queue_network_media_policy_outcome(
                MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated,
            );
            return;
        }
        if origin == AuthoritativePathObservationOrigin::PathEvent
            && self.embedded_network_media_options_belong_to_pending_load()
        {
            // Until a matching target establishes the pending load's generation, any event-time
            // network path can belong to the file being replaced. A later property poll can
            // safely establish that a mismatched external path is still authoritative.
            return;
        }
        let embedded_generation_is_current = self
            .network_options
            .network_media_options_embedded_load
            .as_ref()
            .is_some_and(|embedded| Some(embedded.media_generation) == media_generation);
        // Poll-time mismatches apply to the current external path but retain a pending embedded
        // marker in case Sorotte's requested target appears later. Only an orphaned marker is
        // obsolete here.
        if embedded_generation_is_current
            && !self.embedded_network_media_options_belong_to_pending_load()
        {
            self.network_options.network_media_options_embedded_load = None;
        }

        let attempt_id = self.begin_network_media_options_apply_attempt(media_generation, path);

        let outcome = match self.apply_network_media_options_for_attempt(path, attempt_id) {
            Ok(true) => {
                self.record_unverified_network_media_options_applied();
                MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated
            }
            Ok(false) => return,
            Err(error) => MpvNetworkMediaPolicyOutcome::Failed(error),
        };
        self.queue_network_media_policy_outcome(outcome);
    }

    pub(super) fn set_network_media_policy_state(&mut self, state: MpvNetworkMediaPolicyState) {
        if self.network_options.network_media_options_policy_state != state {
            self.network_options.network_media_options_policy_state = state;
            self.bump_network_options_runtime_health_revision();
        }
    }

    pub(super) fn queue_network_media_policy_outcome(
        &mut self,
        outcome: MpvNetworkMediaPolicyOutcome,
    ) {
        let state = match &outcome {
            MpvNetworkMediaPolicyOutcome::NoActiveMedia => {
                MpvNetworkMediaPolicyState::NoActiveMedia
            }
            MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged => {
                MpvNetworkMediaPolicyState::LocalMediaUnchanged
            }
            MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                MpvNetworkMediaPolicyState::NetworkMediaUpdated
            }
            MpvNetworkMediaPolicyOutcome::Failed(error) => {
                MpvNetworkMediaPolicyState::Failed(error.to_string())
            }
        };
        self.set_network_media_policy_state(state);
        if self
            .network_options
            .pending_network_media_policy_outcomes
            .len()
            == MAX_PENDING_NETWORK_MEDIA_OPTIONS_TRANSITION_OUTCOMES
        {
            self.network_options
                .pending_network_media_policy_outcomes
                .pop_front();
        }
        let sequence = self.next_network_options_event_sequence();
        self.network_options
            .pending_network_media_policy_outcomes
            .push_back(SequencedNetworkOptionsEvent {
                sequence,
                value: outcome,
            });
    }

    pub(super) fn reset_network_media_policy_diagnostics(&mut self) {
        self.network_options.network_media_options_application_state = None;
        self.network_options
            .network_media_options_diagnostic_load_sequence = None;
        self.network_options
            .network_media_options_verification_complete = false;
        self.network_options
            .network_media_options_option_results
            .clear();
        self.network_options
            .network_media_options_effective_cache_options
            .clear();
    }

    pub(super) fn canonical_network_media_diagnostic_value(
        name: &str,
        value: &str,
    ) -> Option<String> {
        let trimmed = value.trim();
        match name {
            "cache" => match trimmed.to_ascii_lowercase().as_str() {
                "yes" => Some("yes".to_owned()),
                "no" => Some("no".to_owned()),
                "auto" => Some("auto".to_owned()),
                "auto-safe" => Some("auto-safe".to_owned()),
                _ => None,
            },
            "cache-pause" | "cache-pause-initial" | "cache-on-disk" => {
                Self::normalize_mpv_boolean(trimmed)
                    .map(|enabled| if enabled { "yes" } else { "no" }.to_owned())
            }
            "cache-pause-wait" | "cache-secs" => {
                let number = trimmed.parse::<f64>().ok()?;
                (number.is_finite() && number >= 0.0).then(|| number.to_string())
            }
            "demuxer-max-bytes" | "demuxer-max-back-bytes" => {
                let bytes = Self::parse_mpv_byte_quantity(trimmed)?;
                (bytes <= u64::MAX as f64).then(|| bytes.to_string())
            }
            _ => None,
        }
    }

    pub(super) fn send_network_media_loadfile(&mut self, path: &str) -> Result<(), PlayerError> {
        let options = Value::Object(self.network_media_options_map());
        self.send_ipc_command_if_attached_without_draining_events(json!([
            MPV_COMMAND_LOADFILE,
            path,
            MPV_LOADFILE_REPLACE,
            -1,
            options
        ]))
    }
}
