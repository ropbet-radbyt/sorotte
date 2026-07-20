use std::{
    hash::BuildHasher,
    process::Command,
    time::{Duration, Instant, SystemTime},
};

use sorotte_plex::PlexServerConnectionKind;

use crate::app::shell_state::GuiPersistedSettingsPatch;

use super::super::runtime_bridge::{GuiPlexPlaylistJobCancellationReason, GuiRuntimeRequest};
use super::*;

const PLEX_AUTH_AUTO_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PLEX_WATCH_SYNC_PUMP_INTERVAL: Duration = Duration::from_secs(1);
const PLEX_WATCH_CACHE_FILE_NAME: &str = "plex-watch-cache.json";

impl GuiPlexServerDiscoveryCoordinator {
    fn operation_context(&self, settings: &StoredClientSettingsMvp) -> GuiPlexOperationContext {
        let resolved = ClientConfig::resolve(settings).config;
        let plugin_enabled = resolved.plugins.plex_enabled;
        let config = resolved.plex;
        GuiPlexOperationContext {
            identity_generation: self.identity_generation,
            user_token_fingerprint: config
                .user_token
                .as_ref()
                .map(|token| self.token_fingerprint(token)),
            selected_server_token_fingerprint: config
                .selected_server_token
                .as_ref()
                .map(|token| self.token_fingerprint(token)),
            selected_server_id: config.selected_server_id,
            selected_server_url: config.selected_server_url,
            plugin_enabled,
            sync_enabled: config.sync_enabled,
            streaming_enabled: config.streaming_enabled,
        }
    }

    fn token_fingerprint(&self, token: &sorotte_secret::SecretValue) -> u64 {
        self.token_fingerprint_state.hash_one(token)
    }

    fn invalidate_operation_context(&mut self) {
        self.identity_generation = self.identity_generation.wrapping_add(1).max(1);
        self.invalidate();
    }

    fn begin_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.active = None;
        self.generation
    }

    fn install(
        &mut self,
        generation: u64,
        operation_context: GuiPlexOperationContext,
        context: GuiPlexServerRefreshContext,
        receiver: mpsc::Receiver<GuiPlexServerDiscoveryWorkerResult>,
    ) {
        if generation == self.generation {
            self.active = Some(GuiPlexServerDiscoveryJob {
                generation,
                operation_context,
                context,
                receiver,
            });
        }
    }

    fn take_active(&mut self) -> Option<GuiPlexServerDiscoveryJob> {
        self.active.take()
    }

    fn restore_if_current(&mut self, job: GuiPlexServerDiscoveryJob) {
        if job.generation == self.generation && self.active.is_none() {
            self.active = Some(job);
        }
    }

    fn accepts(
        &self,
        job: &GuiPlexServerDiscoveryJob,
        result: &GuiPlexServerDiscoveryWorkerResult,
        current_operation_context: &GuiPlexOperationContext,
    ) -> bool {
        job.generation == self.generation
            && result.generation == job.generation
            && result.operation_context == job.operation_context
            && &result.operation_context == current_operation_context
            && result.context == job.context
    }

    pub(super) fn invalidate(&mut self) {
        let _ = self.begin_generation();
    }
}

impl std::fmt::Debug for GuiPlexServerDiscoveryWorkerResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiPlexServerDiscoveryWorkerResult")
            .field("generation", &self.generation)
            .field("operation_context", &self.operation_context)
            .field("context", &self.context)
            .field("succeeded", &self.result.is_ok())
            .finish()
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn plex_operation_context(
        &self,
        settings: &StoredClientSettingsMvp,
    ) -> GuiPlexOperationContext {
        self.plex_server_discovery.operation_context(settings)
    }

    fn begin_plex_playlist_job_generation(&mut self) -> u64 {
        self.plex_playlist_job_generation =
            self.plex_playlist_job_generation.wrapping_add(1).max(1);
        self.plex_playlist_job_generation
    }

    pub(super) fn cancel_plex_playlist_jobs(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        reason: GuiPlexPlaylistJobCancellationReason,
    ) {
        let _ = self.begin_plex_playlist_job_generation();
        self.plex_playlist_search_job = None;
        self.plex_playlist_resolve_job = None;
        if reason == GuiPlexPlaylistJobCancellationReason::OperationContextInvalidated
            && projected_state.plex_playlist_search.is_some()
        {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::CancelPlexPlaylistSearch],
            );
        }
    }

    pub(super) fn invalidate_plex_operation_context(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        self.cancel_plex_playlist_jobs(
            handle,
            projected_state,
            GuiPlexPlaylistJobCancellationReason::OperationContextInvalidated,
        );
        self.plex_server_discovery.invalidate_operation_context();
        self.plex_sync_engine = None;
        self.plex_sync_rx = None;
        self.plex_sync_next_tick_due_at = None;
        self.clear_plex_stream_resolution_state();
    }

    pub(super) fn invalidate_plex_operation_context_if_settings_changed(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        previous: &StoredClientSettingsMvp,
        next: &StoredClientSettingsMvp,
    ) {
        if self.plex_operation_context(previous) != self.plex_operation_context(next) {
            self.invalidate_plex_operation_context(handle, projected_state);
        }
    }

    #[cfg(test)]
    fn apply_authenticated_plex_account(
        &mut self,
        settings: &mut StoredClientSettingsMvp,
        token: sorotte_secret::SecretValue,
    ) {
        let account_changed = settings.plex_user_token.as_ref() != Some(&token);
        settings.plex_user_token = Some(token);
        settings.plex_sync_enabled.get_or_insert(false);
        if account_changed {
            // A selected server token is scoped to the account that supplied
            // it. Keep server-scoped work suspended until login discovery has
            // selected a server visible to the newly authenticated account.
            settings.plex_selected_server_id = None;
            settings.plex_selected_server_url = None;
            settings.plex_selected_server_token = None;
            self.plex_servers.clear();
            self.plex_server_reachability.clear();
        }
    }

    pub(super) fn handle_start_plex_auth_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        if self.plex_auth_start_rx.is_some() || self.plex_auth_poll_rx.is_some() {
            return true;
        }
        let client = match self.ensure_plex_client() {
            Ok(client) => client.clone(),
            Err(message) => {
                self.apply_plex_error(handle, projected_state, message);
                return true;
            }
        };
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-plex-auth-start".to_owned())
            .spawn(move || {
                let result = PlexAuthService::new(client)
                    .start()
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            }) {
            Ok(_thread) => {
                self.plex_auth_start_rx = Some(rx);
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
            }
            Err(error) => self.apply_plex_error(
                handle,
                projected_state,
                format!("Failed to start Plex login worker: {error}"),
            ),
        }
        true
    }

    pub(super) fn handle_poll_plex_auth_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        self.poll_plex_auth(handle, projected_state, true);
        true
    }

    pub(super) fn pump_plex_auth_poll(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return;
        }
        self.drain_plex_auth_start(handle, projected_state);
        self.drain_plex_auth_poll(handle, projected_state);
        if self.plex_auth_start_rx.is_some() || self.plex_auth_poll_rx.is_some() {
            return;
        }
        let Some(due_at) = self.plex_auth_poll_due_at else {
            return;
        };
        if Instant::now() < due_at {
            return;
        }
        self.poll_plex_auth(handle, projected_state, false);
    }

    fn drain_plex_auth_start(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(rx) = self.plex_auth_start_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(session)) => {
                let auth_url = session.auth_url.clone();
                self.plex_auth_session = Some(session);
                self.schedule_next_plex_auth_poll();
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
                let mut actions = vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message:
                        "Plex login started. Complete the browser prompt; this panel will update automatically."
                            .to_owned(),
                }];
                if !cfg!(test)
                    && let Err(error) = open_system_url(&auth_url)
                {
                    actions.push(GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Warning,
                        message: format!(
                            "Could not open Plex login URL automatically: {error}. The URL is shown in the Plex panel."
                        ),
                    });
                }
                Self::push_actions_and_project(handle, projected_state, actions);
            }
            Ok(Err(error)) => self.apply_plex_error(
                handle,
                projected_state,
                format!("Failed to start Plex login: {error}"),
            ),
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_auth_start_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => self.apply_plex_error(
                handle,
                projected_state,
                "Plex login worker stopped before returning a result.".to_owned(),
            ),
        }
    }

    pub(super) fn pump_startup_plex_server_refresh(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return;
        }
        let settings = projected_state.saved_configuration.clone();
        if !self.startup_plex_server_refresh_attempted
            && settings
                .plex_user_token
                .as_ref()
                .is_some_and(|token| !token.is_blank())
        {
            self.startup_plex_server_refresh_attempted = true;
            if let Some(url) = settings.plex_selected_server_url.as_deref() {
                self.plex_server_reachability.insert(
                    plex_server_reachability_key(url),
                    GuiPlexServerReachability::Checking,
                );
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
            }
            if let Err(error) = self
                .start_plex_server_refresh_worker(&settings, GuiPlexServerRefreshContext::Startup)
            {
                self.apply_plex_error(
                    handle,
                    projected_state,
                    format!("Failed to start Plex server refresh at startup: {error}"),
                );
            }
        }
    }

    pub(super) fn pump_plex_server_refresh(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return;
        }
        let Some(job) = self.plex_server_discovery.take_active() else {
            return;
        };
        match job.receiver.try_recv() {
            Ok(worker_result) => {
                let mut settings = projected_state.saved_configuration.clone();
                let previous_settings = settings.clone();
                let current_operation_context = self.plex_operation_context(&settings);
                if !self.plex_server_discovery.accepts(
                    &job,
                    &worker_result,
                    &current_operation_context,
                ) {
                    return;
                }
                match worker_result.result {
                    Ok(outcome) => {
                        let context = worker_result.context;
                        self.apply_plex_server_refresh_outcome(outcome);
                        let selection_changed = reconcile_plex_server_selection(
                            &mut settings,
                            &self.plex_servers,
                            true,
                        );
                        let mut selection_persisted = true;
                        if selection_changed {
                            let patch = GuiPersistedSettingsPatch::PlexServerSelected {
                                machine_identifier: settings
                                    .plex_selected_server_id
                                    .clone()
                                    .expect("reconciled Plex server has an identifier"),
                                uri: settings
                                    .plex_selected_server_url
                                    .clone()
                                    .expect("reconciled Plex server has a URL"),
                                access_token: settings
                                    .plex_selected_server_token
                                    .clone()
                                    .expect("reconciled Plex server has an access token"),
                            };
                            if let Some(persisted_settings) = self.persist_plex_patch(
                                handle,
                                projected_state,
                                patch,
                                "Automatically selected Plex server was not saved",
                            ) {
                                settings = persisted_settings;
                                self.invalidate_plex_operation_context_if_settings_changed(
                                    handle,
                                    projected_state,
                                    &previous_settings,
                                    &settings,
                                );
                            } else {
                                settings = previous_settings.clone();
                                selection_persisted = false;
                            }
                        }
                        let has_selected_server = settings.plex_selected_server_url.is_some();
                        if selection_persisted {
                            self.sync_plex_runtime_snapshot(handle, projected_state, None);
                        }
                        let empty_message = match context {
                            GuiPlexServerRefreshContext::Startup => None,
                            GuiPlexServerRefreshContext::Manual => Some(
                                "No reachable Plex Media Servers were returned for this account.",
                            ),
                            GuiPlexServerRefreshContext::Login => Some(
                                "Plex login succeeded, but no reachable Plex Media Servers were returned for this account.",
                            ),
                        };
                        if self.plex_servers.is_empty()
                            && !has_selected_server
                            && let Some(message) = empty_message
                        {
                            Self::push_actions_and_project(
                                handle,
                                projected_state,
                                vec![GuiShellAction::PushTransientNotification {
                                    level: GuiTransientNotificationLevel::Warning,
                                    message: message.to_owned(),
                                }],
                            );
                        }
                    }
                    Err(error) => match worker_result.context {
                        GuiPlexServerRefreshContext::Startup => {
                            if let Some(url) = settings.plex_selected_server_url.as_deref() {
                                self.plex_server_reachability.insert(
                                    plex_server_reachability_key(url),
                                    GuiPlexServerReachability::Unreachable,
                                );
                            }
                            self.apply_plex_error(
                                handle,
                                projected_state,
                                format!("Failed to refresh Plex servers at startup: {error}"),
                            );
                        }
                        GuiPlexServerRefreshContext::Manual => self.apply_plex_error(
                            handle,
                            projected_state,
                            format!("Failed to refresh Plex servers: {error}"),
                        ),
                        GuiPlexServerRefreshContext::Login => Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Warning,
                                message: format!(
                                    "Plex login succeeded, but server discovery failed: {error}"
                                ),
                            }],
                        ),
                    },
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_server_discovery.restore_if_current(job);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let current_settings = projected_state.saved_configuration.clone();
                if job.generation == self.plex_server_discovery.generation
                    && job.operation_context == self.plex_operation_context(&current_settings)
                {
                    let message = match job.context {
                        GuiPlexServerRefreshContext::Startup => {
                            "Plex startup server refresh worker stopped before returning a result."
                        }
                        GuiPlexServerRefreshContext::Manual
                        | GuiPlexServerRefreshContext::Login => {
                            "Plex server refresh worker stopped before returning a result."
                        }
                    };
                    self.apply_plex_error(handle, projected_state, message.to_owned());
                }
            }
        }
    }

    fn poll_plex_auth(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        user_initiated: bool,
    ) {
        if self.plex_auth_poll_rx.is_some() {
            return;
        }
        let Some(session) = self.plex_auth_session.clone() else {
            self.plex_auth_poll_due_at = None;
            if user_initiated {
                self.apply_plex_error(
                    handle,
                    projected_state,
                    "No Plex login is currently in progress.".to_owned(),
                );
            }
            return;
        };
        let client = match self.ensure_plex_client() {
            Ok(client) => client.clone(),
            Err(message) => {
                if user_initiated {
                    self.apply_plex_error(handle, projected_state, message);
                } else {
                    self.schedule_next_plex_auth_poll();
                }
                return;
            }
        };
        self.plex_auth_poll_due_at = None;
        let pin_id = session.pin_id;
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-plex-auth-poll".to_owned())
            .spawn(move || {
                let result = PlexAuthService::new(client)
                    .poll(pin_id)
                    .map_err(|error| error.to_string());
                let _ = tx.send((user_initiated, result));
            }) {
            Ok(_thread) => {
                self.plex_auth_poll_rx = Some(rx);
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
            }
            Err(error) => {
                if user_initiated {
                    self.apply_plex_error(
                        handle,
                        projected_state,
                        format!("Failed to start Plex login check: {error}"),
                    );
                } else {
                    self.schedule_next_plex_auth_poll();
                }
            }
        }
    }

    fn drain_plex_auth_poll(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(rx) = self.plex_auth_poll_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok((user_initiated, Ok(result))) => {
                self.apply_plex_auth_poll_result(handle, projected_state, user_initiated, result)
            }
            Ok((user_initiated, Err(error))) => {
                if user_initiated {
                    self.apply_plex_error(
                        handle,
                        projected_state,
                        format!("Failed to check Plex login: {error}"),
                    );
                } else {
                    self.schedule_next_plex_auth_poll();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_auth_poll_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.schedule_next_plex_auth_poll();
            }
        }
    }

    fn apply_plex_auth_poll_result(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        user_initiated: bool,
        result: PlexAuthPollResult,
    ) {
        let Some(token) = result.auth_token else {
            self.schedule_next_plex_auth_poll();
            self.sync_plex_runtime_snapshot(handle, projected_state, None);
            if user_initiated {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: "Plex login is not complete yet.".to_owned(),
                    }],
                );
            }
            return;
        };
        let previous_settings = projected_state.saved_configuration.clone();
        let account_changed = previous_settings.plex_user_token.as_ref() != Some(&token);
        let patch = GuiPersistedSettingsPatch::PlexAuthenticated {
            user_token: token,
            sync_enabled: previous_settings.plex_sync_enabled.unwrap_or(false),
            clear_selected_server: account_changed,
        };
        let settings = match self.persist_saved_settings_patch(projected_state, &patch) {
            Ok(settings) => settings,
            Err(error) => {
                self.plex_auth_session = None;
                self.plex_auth_poll_due_at = None;
                self.apply_plex_error(
                    handle,
                    projected_state,
                    format!(
                        "Plex login completed, but login was not saved: {error}. Select Connect to retry."
                    ),
                );
                return;
            }
        };
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiPersistedSettingsPatch(patch)],
        );
        self.plex_auth_session = None;
        self.plex_auth_poll_due_at = None;
        if account_changed {
            self.plex_servers.clear();
            self.plex_server_reachability.clear();
        }
        self.invalidate_plex_operation_context_if_settings_changed(
            handle,
            projected_state,
            &previous_settings,
            &settings,
        );
        let refresh_start_error = self
            .start_plex_server_refresh_worker(&settings, GuiPlexServerRefreshContext::Login)
            .err();
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        let mut actions = vec![GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            message: "Plex login complete.".to_owned(),
        }];
        if let Some(error) = refresh_start_error {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: format!(
                    "Plex login succeeded, but server discovery could not start: {error}"
                ),
            });
        }
        Self::push_actions_and_project(handle, projected_state, actions);
    }

    pub(super) fn handle_refresh_plex_servers_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let settings = projected_state.saved_configuration.clone();
        if let Err(error) =
            self.start_plex_server_refresh_worker(&settings, GuiPlexServerRefreshContext::Manual)
        {
            self.apply_plex_error(
                handle,
                projected_state,
                format!("Failed to refresh Plex servers: {error}"),
            );
            return true;
        }
        if let Some(url) = settings.plex_selected_server_url.as_deref() {
            self.plex_server_reachability.insert(
                plex_server_reachability_key(url),
                GuiPlexServerReachability::Checking,
            );
        }
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_select_plex_server_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        machine_identifier: String,
        uri: String,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let Some(server) = self
            .plex_servers
            .iter()
            .find(|server| server.machine_identifier == machine_identifier && server.uri == uri)
            .cloned()
        else {
            self.apply_plex_error(
                handle,
                projected_state,
                "Selected Plex server is no longer available.".to_owned(),
            );
            return true;
        };
        let previous_settings = projected_state.saved_configuration.clone();
        let patch = GuiPersistedSettingsPatch::PlexServerSelected {
            machine_identifier: server.machine_identifier,
            uri: server.uri,
            access_token: server.access_token,
        };
        let settings = match self.persist_plex_patch(
            handle,
            projected_state,
            patch,
            "Selected Plex server was not saved",
        ) {
            Some(settings) => settings,
            None => return true,
        };
        self.invalidate_plex_operation_context_if_settings_changed(
            handle,
            projected_state,
            &previous_settings,
            &settings,
        );
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_toggle_plex_sync_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let previous_settings = projected_state.saved_configuration.clone();
        let patch = GuiPersistedSettingsPatch::PlexSyncEnabled(enabled);
        let Some(settings) = self.persist_plex_patch(
            handle,
            projected_state,
            patch,
            "Plex sync setting was not saved",
        ) else {
            return true;
        };
        self.invalidate_plex_operation_context_if_settings_changed(
            handle,
            projected_state,
            &previous_settings,
            &settings,
        );
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_toggle_plex_streaming_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let previous_settings = projected_state.saved_configuration.clone();
        let patch = GuiPersistedSettingsPatch::PlexStreamingEnabled(enabled);
        let Some(settings) = self.persist_plex_patch(
            handle,
            projected_state,
            patch,
            "Plex streaming setting was not saved",
        ) else {
            return true;
        };
        self.invalidate_plex_operation_context_if_settings_changed(
            handle,
            projected_state,
            &previous_settings,
            &settings,
        );
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_disconnect_plex_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(_settings) = self.persist_plex_patch(
            handle,
            projected_state,
            GuiPersistedSettingsPatch::PlexDisconnected,
            "Plex disconnect was not saved",
        ) else {
            return true;
        };
        self.plex_auth_session = None;
        self.plex_auth_start_rx = None;
        self.plex_auth_poll_rx = None;
        self.plex_auth_poll_due_at = None;
        self.plex_servers.clear();
        self.plex_server_reachability.clear();
        self.invalidate_plex_operation_context(handle, projected_state);
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_search_selected_plex_server_media_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        query: String,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let picker = projected_state
            .plex_playlist_search
            .get_or_insert_with(Default::default);
        picker.query.clone_from(&query);
        picker.searching = true;
        picker.adding_rating_key = None;
        picker.error = None;
        let id = self.begin_plex_playlist_job_generation();
        self.plex_playlist_search_job = None;
        // Submitting a new search clears the picker’s pending add state, so
        // any resolve owned by the previous results is superseded as well.
        self.plex_playlist_resolve_job = None;
        let client = match self.ensure_plex_client() {
            Ok(client) => client.clone(),
            Err(error) => {
                self.complete_plex_playlist_search_with_error(
                    handle,
                    projected_state,
                    query,
                    error,
                );
                return true;
            }
        };
        let settings = projected_state.saved_configuration.clone();
        let operation_context = self.plex_operation_context(&settings);
        let config = plex_config_from_settings(&settings);
        let worker_query = query.clone();
        let worker_operation_context = operation_context.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-plex-playlist-search".to_owned())
            .spawn(move || {
                let result = PlexLibraryService::new(&client)
                    .search_selected(&config, &worker_query, 25)
                    .map(|results| {
                        results
                            .into_iter()
                            .map(GuiPlexPlaylistSearchResult::from)
                            .collect::<Vec<_>>()
                    })
                    .map_err(|error| error.to_string());
                let _ = tx.send(GuiPlexPlaylistSearchWorkerResult {
                    id,
                    operation_context: worker_operation_context,
                    query: worker_query,
                    result,
                });
            }) {
            Ok(_thread) => {
                self.plex_playlist_search_job = Some(GuiActivePlexPlaylistSearchJob {
                    id,
                    operation_context,
                    query,
                    result_rx: rx,
                });
            }
            Err(error) => self.complete_plex_playlist_search_with_error(
                handle,
                projected_state,
                query,
                format!("Failed to start Plex playlist search worker: {error}"),
            ),
        }
        true
    }

    pub(super) fn handle_resolve_plex_playlist_item_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        rating_key: String,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::Plex,
            );
            return true;
        }
        let picker = projected_state
            .plex_playlist_search
            .get_or_insert_with(Default::default);
        picker.adding_rating_key = Some(rating_key.clone());
        picker.error = None;
        let id = self.begin_plex_playlist_job_generation();
        self.plex_playlist_resolve_job = None;
        let client = match self.ensure_plex_client() {
            Ok(client) => client.clone(),
            Err(error) => {
                self.complete_plex_playlist_resolve_with_error(
                    handle,
                    projected_state,
                    rating_key,
                    error,
                );
                return true;
            }
        };
        let settings = projected_state.saved_configuration.clone();
        let operation_context = self.plex_operation_context(&settings);
        let config = plex_config_from_settings(&settings);
        let worker_rating_key = rating_key.clone();
        let worker_operation_context = operation_context.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-plex-playlist-resolve".to_owned())
            .spawn(move || {
                let result = PlexLibraryService::new(&client)
                    .playlist_uri(&config, &worker_rating_key)
                    .map(|uri| format_plex_playlist_uri(&uri))
                    .map_err(|error| error.to_string());
                let _ = tx.send(GuiPlexPlaylistResolveWorkerResult {
                    id,
                    operation_context: worker_operation_context,
                    rating_key: worker_rating_key,
                    result,
                });
            }) {
            Ok(_thread) => {
                self.plex_playlist_resolve_job = Some(GuiActivePlexPlaylistResolveJob {
                    id,
                    operation_context,
                    rating_key,
                    result_rx: rx,
                });
            }
            Err(error) => self.complete_plex_playlist_resolve_with_error(
                handle,
                projected_state,
                rating_key,
                format!("Failed to start Plex playlist resolve worker: {error}"),
            ),
        }
        true
    }

    fn plex_watch_event_for_current_player(&self) -> Option<PlexWatchEvent> {
        if !self.player_local_file_ready_for_attached_sync() {
            return None;
        }
        self.player_local_file.clone().map(|file| {
            let mut event = PlexWatchEvent::new(file).with_changed_at(SystemTime::now());
            if let Some(position) = self.player_position_seconds {
                event = event.with_position_seconds(position);
            }
            if let Some(paused) = self.player_paused {
                event = event.with_paused(paused);
            }
            event
        })
    }

    pub(super) fn sync_plex_watch_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            self.plex_sync_rx = None;
            self.plex_sync_next_tick_due_at = None;
            return false;
        }
        if self.drain_plex_sync_worker(handle, projected_state) {
            // Give a stream resolution that was waiting on this engine a chance
            // to start before the periodic sync loop takes ownership again.
            return true;
        }
        if self.plex_sync_rx.is_some() {
            return false;
        }
        let settings = projected_state.saved_configuration.clone();
        let operation_context = self.plex_operation_context(&settings);
        let config = plex_config_from_settings(&settings);
        if !config.enabled || !config.has_selected_server() {
            self.plex_sync_next_tick_due_at = None;
            if let Some(engine) = self.plex_sync_engine.as_mut() {
                engine.set_config(config);
            }
            self.sync_plex_runtime_snapshot(handle, projected_state, None);
            return false;
        }
        let now = Instant::now();
        if let Some(due_at) = self.plex_sync_next_tick_due_at
            && now < due_at
        {
            return false;
        }
        if self.plex_stream_resolution_owns_cache_snapshot() {
            return false;
        }

        let event = self.plex_watch_event_for_current_player();
        let cache_path = self.plex_cache_path();
        let engine = match self.take_plex_sync_engine(config) {
            Ok(engine) => engine,
            Err(message) => {
                self.apply_plex_error(handle, projected_state, message);
                return false;
            }
        };
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("sorotte-gui-plex-watch-sync".to_owned())
            .spawn(move || {
                let mut engine = engine;
                let before = engine.cache().clone();
                let status = engine.tick(event, SystemTime::now());
                let staged_cache_write = if engine.cache() != &before {
                    cache_path.map(|path| {
                        engine
                            .cache()
                            .stage_to_path(&path)
                            .map_err(|error| format!("Failed to stage Plex match cache: {error}"))
                    })
                } else {
                    None
                };
                let _ = tx.send(GuiPlexSyncWorkerResult {
                    operation_context,
                    staged_cache_write,
                    engine,
                    status,
                });
            }) {
            Ok(_thread) => {
                self.plex_sync_rx = Some(rx);
                self.plex_sync_next_tick_due_at = Some(now + PLEX_WATCH_SYNC_PUMP_INTERVAL);
            }
            Err(error) => {
                self.apply_plex_error(
                    handle,
                    projected_state,
                    format!("Failed to start Plex sync worker: {error}"),
                );
            }
        }
        false
    }

    pub(super) fn pump_plex_playlist_workers(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return;
        }
        self.drain_plex_playlist_search_worker(handle, projected_state);
        self.drain_plex_playlist_resolve_worker(handle, projected_state);
    }

    pub(in crate::app::runtime_owner) fn ensure_plex_client(
        &mut self,
    ) -> Result<&PlexHttpClient, String> {
        if self.plex_client.is_none() {
            self.plex_client = Some(
                PlexHttpClient::new("sorotte-gui")
                    .map_err(|error| format!("Failed to create Plex HTTP client: {error}"))?,
            );
        }
        self.plex_client
            .as_ref()
            .ok_or_else(|| "Failed to create Plex HTTP client.".to_owned())
    }

    pub(in crate::app::runtime_owner) fn take_plex_sync_engine(
        &mut self,
        config: PlexClientConfig,
    ) -> Result<PlexSyncEngine<PlexHttpClient>, String> {
        if self.plex_sync_engine.is_none() {
            let client = self.ensure_plex_client()?.clone();
            let cache = self.load_persisted_plex_match_cache()?;
            self.plex_sync_engine = Some(PlexSyncEngine::new(config.clone(), client, cache));
        }
        let mut engine = self
            .plex_sync_engine
            .take()
            .ok_or_else(|| "Failed to create Plex sync engine.".to_owned())?;
        engine.set_config(config);
        Ok(engine)
    }

    fn drain_plex_sync_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(rx) = self.plex_sync_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result)
                if result.operation_context
                    == self.plex_operation_context(&projected_state.saved_configuration) =>
            {
                let cache_save_error = result.staged_cache_write.and_then(|staged| match staged {
                    Ok(staged) => staged
                        .commit()
                        .err()
                        .map(|error| format!("Failed to commit Plex match cache: {error}")),
                    Err(error) => Some(error),
                });
                self.plex_sync_engine = Some(result.engine);
                if let Some(error) = cache_save_error {
                    self.apply_plex_error(handle, projected_state, error);
                } else {
                    self.sync_plex_runtime_snapshot(handle, projected_state, Some(result.status));
                }
                true
            }
            Ok(_stale_result) => true,
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_sync_rx = Some(rx);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.apply_plex_error(
                    handle,
                    projected_state,
                    "Plex sync worker stopped before returning a result.".to_owned(),
                );
                true
            }
        }
    }

    fn drain_plex_playlist_search_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(job) = self.plex_playlist_search_job.take() else {
            return;
        };
        let Some((restore_job, actions)) = handle.commit_if_client_command_queue_idle(|| {
            let (restore_job, actions) = match job.result_rx.try_recv() {
                Ok(result) => {
                    let current_context =
                        self.plex_operation_context(&projected_state.saved_configuration);
                    let picker_accepts = projected_state
                        .plex_playlist_search
                        .as_ref()
                        .is_some_and(|search| search.searching && search.query == job.query);
                    if result.id != job.id
                        || result.operation_context != job.operation_context
                        || result.operation_context != current_context
                        || result.query != job.query
                        || !picker_accepts
                    {
                        (false, Vec::new())
                    } else {
                        let (results, error) = match result.result {
                            Ok(results) => (results, None),
                            Err(error) => (Vec::new(), Some(error)),
                        };
                        (
                            false,
                            vec![GuiShellAction::CompletePlexPlaylistSearch {
                                query: result.query,
                                results,
                                error,
                            }],
                        )
                    }
                }
                Err(mpsc::TryRecvError::Empty) => (true, Vec::new()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    let current_context =
                        self.plex_operation_context(&projected_state.saved_configuration);
                    let picker_accepts = projected_state
                        .plex_playlist_search
                        .as_ref()
                        .is_some_and(|search| search.searching && search.query == job.query);
                    let actions = if job.operation_context == current_context && picker_accepts {
                        vec![GuiShellAction::CompletePlexPlaylistSearch {
                            query: job.query.clone(),
                            results: Vec::new(),
                            error: Some(
                                "Plex playlist search worker stopped before returning a result."
                                    .to_owned(),
                            ),
                        }]
                    } else {
                        Vec::new()
                    };
                    (false, actions)
                }
            };
            for action in actions.iter().cloned() {
                let _ = projected_state.apply(action);
            }
            ((restore_job, actions), Vec::new())
        }) else {
            self.plex_playlist_search_job = Some(job);
            return;
        };
        if restore_job {
            self.plex_playlist_search_job = Some(job);
        }
        handle.push_actions(actions);
    }

    fn drain_plex_playlist_resolve_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(job) = self.plex_playlist_resolve_job.take() else {
            return;
        };
        let Some((restore_job, actions)) = handle.commit_if_client_command_queue_idle(|| {
            let (restore_job, actions, requests) = match job.result_rx.try_recv() {
                Ok(result) => {
                    let current_context =
                        self.plex_operation_context(&projected_state.saved_configuration);
                    let picker_accepts = projected_state
                        .plex_playlist_search
                        .as_ref()
                        .and_then(|search| search.adding_rating_key.as_deref())
                        == Some(job.rating_key.as_str());
                    if result.id != job.id
                        || result.operation_context != job.operation_context
                        || result.operation_context != current_context
                        || result.rating_key != job.rating_key
                        || !picker_accepts
                    {
                        (false, Vec::new(), Vec::new())
                    } else {
                        match result.result {
                            Ok(playlist_uri) => {
                                let requests = if projected_state
                                    .current_shared_playlist_entries()
                                    .iter()
                                    .any(|entry| entry == &playlist_uri)
                                {
                                    Vec::new()
                                } else {
                                    vec![GuiRuntimeRequest::QueuePlaylistEntry {
                                        entry: playlist_uri.clone(),
                                        select_after_queue: false,
                                    }]
                                };
                                (
                                    false,
                                    vec![
                                        GuiShellAction::AppendSharedPlaylistEntries(vec![
                                            playlist_uri,
                                        ]),
                                        GuiShellAction::CompletePlexPlaylistItemResolve {
                                            rating_key: result.rating_key,
                                            error: None,
                                        },
                                    ],
                                    requests,
                                )
                            }
                            Err(error) => (
                                false,
                                vec![GuiShellAction::CompletePlexPlaylistItemResolve {
                                    rating_key: result.rating_key,
                                    error: Some(error),
                                }],
                                Vec::new(),
                            ),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => (true, Vec::new(), Vec::new()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    let current_context =
                        self.plex_operation_context(&projected_state.saved_configuration);
                    let picker_accepts = projected_state
                        .plex_playlist_search
                        .as_ref()
                        .and_then(|search| search.adding_rating_key.as_deref())
                        == Some(job.rating_key.as_str());
                    let actions = if job.operation_context == current_context && picker_accepts {
                        vec![GuiShellAction::CompletePlexPlaylistItemResolve {
                            rating_key: job.rating_key.clone(),
                            error: Some(
                                "Plex playlist resolve worker stopped before returning a result."
                                    .to_owned(),
                            ),
                        }]
                    } else {
                        Vec::new()
                    };
                    (false, actions, Vec::new())
                }
            };
            for action in actions.iter().cloned() {
                let _ = projected_state.apply(action);
            }
            ((restore_job, actions), requests)
        }) else {
            self.plex_playlist_resolve_job = Some(job);
            return;
        };
        if restore_job {
            self.plex_playlist_resolve_job = Some(job);
        }
        handle.push_actions(actions);
    }

    fn complete_plex_playlist_search_with_error(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        query: String,
        error: String,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::CompletePlexPlaylistSearch {
                query,
                results: Vec::new(),
                error: Some(error),
            }],
        );
    }

    fn complete_plex_playlist_resolve_with_error(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        rating_key: String,
        error: String,
    ) {
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::CompletePlexPlaylistItemResolve {
                rating_key,
                error: Some(error),
            }],
        );
    }

    fn start_plex_server_refresh_worker(
        &mut self,
        settings: &StoredClientSettingsMvp,
        context: GuiPlexServerRefreshContext,
    ) -> Result<(), String> {
        if context != GuiPlexServerRefreshContext::Startup {
            self.startup_plex_server_refresh_attempted = true;
        }
        let generation = self.plex_server_discovery.begin_generation();
        let operation_context = self.plex_operation_context(settings);
        settings
            .plex_user_token
            .as_ref()
            .filter(|token| !token.is_blank())
            .ok_or_else(|| "Plex login is required before servers can be refreshed.".to_owned())?;
        let client = self.ensure_plex_client().cloned()?;
        let settings = settings.clone();
        let (tx, rx) = mpsc::channel();
        let worker_operation_context = operation_context.clone();
        std::thread::Builder::new()
            .name("sorotte-gui-plex-server-refresh".to_owned())
            .spawn(move || {
                let result = refresh_plex_servers_and_reachability(
                    PlexDiscoveryService::new(client),
                    &settings,
                );
                let _ = tx.send(GuiPlexServerDiscoveryWorkerResult {
                    generation,
                    operation_context: worker_operation_context,
                    context,
                    result,
                });
            })
            .map_err(|error| error.to_string())?;
        self.plex_server_discovery
            .install(generation, operation_context, context, rx);
        Ok(())
    }

    fn apply_plex_server_refresh_outcome(&mut self, outcome: GuiPlexServerRefreshOutcome) {
        self.plex_servers = outcome.servers;
        self.plex_server_reachability = outcome.reachability;
    }

    fn persist_plex_patch(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        patch: GuiPersistedSettingsPatch,
        failure_context: &str,
    ) -> Option<StoredClientSettingsMvp> {
        let settings = match self.persist_saved_settings_patch(projected_state, &patch) {
            Ok(settings) => settings,
            Err(error) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Warning,
                        message: format!("{failure_context}: {error}"),
                    }],
                );
                return None;
            }
        };
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiPersistedSettingsPatch(patch)],
        );
        Some(settings)
    }

    fn sync_plex_runtime_snapshot(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        status: Option<PlexSyncStatus>,
    ) {
        let settings = projected_state.saved_configuration.clone();
        let status = status.or_else(|| self.plex_sync_engine.as_ref().map(PlexSyncEngine::status));
        let snapshot = self.plex_snapshot_from_settings_and_status(&settings, status.as_ref());
        if GuiPlexRuntimeSnapshot::from(&projected_state.plex) != snapshot {
            self.plex_runtime_snapshot = snapshot.clone();
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::ApplyGuiPlexRuntimeSnapshot(snapshot)],
            );
        } else {
            self.plex_runtime_snapshot = snapshot;
        }
    }

    fn plex_snapshot_from_settings_and_status(
        &self,
        settings: &StoredClientSettingsMvp,
        status: Option<&PlexSyncStatus>,
    ) -> GuiPlexRuntimeSnapshot {
        let plex = ClientConfig::resolve(settings).config.plex;
        let selected_server_id = plex.selected_server_id.clone();
        let selected_server_url = plex.selected_server_url.clone();
        let authenticated = plex.user_token.is_some();
        let authenticating = self.plex_auth_session.is_some()
            || self.plex_auth_start_rx.is_some()
            || self.plex_auth_poll_rx.is_some();
        let status_label = status
            .map(|status| plex_sync_state_label(status.state).to_owned())
            .unwrap_or_else(|| {
                if authenticating {
                    "authenticating".to_owned()
                } else if authenticated {
                    "ready".to_owned()
                } else {
                    "disconnected".to_owned()
                }
            });
        let mut server_rows = self
            .plex_servers
            .iter()
            .map(|server| GuiPlexServerRow {
                name: server.name.clone(),
                machine_identifier: server.machine_identifier.clone(),
                uri: server.uri.clone(),
                reachability: self
                    .plex_server_reachability
                    .get(&plex_server_reachability_key(&server.uri))
                    .copied()
                    .unwrap_or_default(),
                connection_kind: server.connection_kind,
                has_local_connection: server.has_local_connection,
                owned: server.owned,
                selected: selected_server_id
                    .as_deref()
                    .is_some_and(|id| id == server.machine_identifier)
                    && selected_server_url
                        .as_deref()
                        .is_some_and(|uri| uri == server.uri),
            })
            .collect::<Vec<_>>();
        if !server_rows.iter().any(|server| server.selected)
            && let Some(uri) = selected_server_url.as_ref()
            && !server_rows.iter().any(|server| &server.uri == uri)
        {
            let machine_identifier = selected_server_id.clone().unwrap_or_default();
            server_rows.push(GuiPlexServerRow {
                name: saved_plex_server_label(selected_server_id.as_deref()),
                machine_identifier,
                uri: uri.clone(),
                reachability: self
                    .plex_server_reachability
                    .get(&plex_server_reachability_key(uri))
                    .copied()
                    .unwrap_or_default(),
                connection_kind: plex_server_connection_kind_from_uri(uri),
                has_local_connection: plex_server_connection_kind_from_uri(uri)
                    == PlexServerConnectionKind::Local,
                owned: true,
                selected: true,
            });
        }

        GuiPlexRuntimeSnapshot {
            enabled: plex.sync_enabled,
            streaming_enabled: plex.streaming_enabled,
            authenticated,
            authenticating,
            auth_code: self
                .plex_auth_session
                .as_ref()
                .map(|session| session.code.clone()),
            auth_url: self
                .plex_auth_session
                .as_ref()
                .map(|session| session.auth_url.clone()),
            selected_server_id: selected_server_id.clone(),
            selected_server_url: selected_server_url.clone(),
            servers: server_rows,
            status: status_label,
            current_item: status
                .and_then(|status| status.current_item.as_ref())
                .map(|item| item.title.clone()),
            last_report: status
                .and_then(|status| status.last_report_at.map(format_plex_report_time)),
            last_error: status.and_then(|status| status.last_error.clone()),
        }
    }

    fn apply_plex_error(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        message: String,
    ) {
        self.plex_runtime_snapshot.last_error = Some(message.clone());
        let mut snapshot =
            self.plex_snapshot_from_settings_and_status(&projected_state.saved_configuration, None);
        snapshot.status = "error".to_owned();
        snapshot.last_error = Some(message.clone());
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::ApplyGuiPlexRuntimeSnapshot(snapshot),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message,
                },
            ],
        );
    }

    fn load_persisted_plex_match_cache(&self) -> Result<PlexMatchCache, String> {
        let Some(cache_path) = self.plex_cache_path() else {
            return Ok(PlexMatchCache::default());
        };
        PlexMatchCache::load_from_path(&cache_path)
            .map_err(|error| format!("Failed to load Plex match cache: {error}"))
    }

    pub(super) fn clear_persisted_plex_match_cache(&self) -> Result<bool, String> {
        let mut changed = false;
        if let Some(path) = self.plex_cache_path() {
            match std::fs::remove_file(&path) {
                Ok(()) => changed = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "failed clearing Plex watch cache {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(changed)
    }

    pub(in crate::app::runtime_owner) fn plex_cache_path(&self) -> Option<PathBuf> {
        self.config_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|parent| parent.join("cache").join(PLEX_WATCH_CACHE_FILE_NAME))
    }

    fn schedule_next_plex_auth_poll(&mut self) {
        self.plex_auth_poll_due_at = Some(Instant::now() + PLEX_AUTH_AUTO_POLL_INTERVAL);
    }
}

pub(in crate::app::runtime_owner) fn plex_config_from_settings(
    settings: &StoredClientSettingsMvp,
) -> PlexClientConfig {
    let plex = ClientConfig::resolve(settings).config.plex;
    PlexClientConfig {
        enabled: plex.sync_enabled,
        streaming_enabled: plex.streaming_enabled,
        user_token: plex.user_token,
        selected_server_id: plex.selected_server_id,
        selected_server_url: plex.selected_server_url,
        selected_server_token: plex.selected_server_token,
    }
}

fn apply_plex_server_to_settings(
    settings: &mut StoredClientSettingsMvp,
    server: &PlexServerConnection,
) {
    settings.plex_selected_server_id = Some(server.machine_identifier.clone());
    settings.plex_selected_server_url = Some(server.uri.clone());
    settings.plex_selected_server_token = Some(server.access_token.clone());
}

fn reconcile_plex_server_selection(
    settings: &mut StoredClientSettingsMvp,
    servers: &[PlexServerConnection],
    select_first_if_missing: bool,
) -> bool {
    if servers.is_empty() {
        return false;
    }
    let selected_id = settings.plex_selected_server_id.as_deref();
    let selected_url = settings.plex_selected_server_url.as_deref();
    if let Some(selected_server) = selected_id.and_then(|id| {
        selected_url.and_then(|url| {
            servers
                .iter()
                .find(|server| server.machine_identifier == id && server.uri == url)
        })
    }) {
        if settings.plex_selected_server_token.as_ref() == Some(&selected_server.access_token) {
            return false;
        }
        apply_plex_server_to_settings(settings, selected_server);
        return true;
    }
    if let Some(id) = selected_id
        && let Some(server) = servers
            .iter()
            .find(|server| server.machine_identifier == id)
            .cloned()
    {
        apply_plex_server_to_settings(settings, &server);
        return true;
    }
    if select_first_if_missing
        && selected_url.is_none()
        && let Some(server) = servers.first()
    {
        apply_plex_server_to_settings(settings, server);
        return true;
    }
    false
}

fn refresh_plex_servers_and_reachability<T>(
    discovery: PlexDiscoveryService<T>,
    settings: &StoredClientSettingsMvp,
) -> Result<GuiPlexServerRefreshOutcome, String>
where
    T: sorotte_plex::discovery::PlexServerDiscoveryTransport,
{
    let token = settings
        .plex_user_token
        .as_ref()
        .filter(|token| !token.is_blank())
        .ok_or_else(|| "Plex login is required before servers can be refreshed.".to_owned())?;
    let servers = discovery
        .discover(token)
        .map_err(|error| error.to_string())?;
    let mut reachability = HashMap::new();
    for server in &servers {
        reachability.insert(
            plex_server_reachability_key(&server.uri),
            verify_plex_server_reachability(&discovery, server),
        );
    }

    if let Some(uri) = settings
        .plex_selected_server_url
        .as_deref()
        .filter(|uri| !uri.trim().is_empty())
        && !servers.iter().any(|server| server.uri == uri)
    {
        let selected_server = PlexServerConnection {
            name: saved_plex_server_label(settings.plex_selected_server_id.as_deref()),
            machine_identifier: settings
                .plex_selected_server_id
                .clone()
                .unwrap_or_else(|| "saved-server".to_owned()),
            uri: uri.to_owned(),
            access_token: settings
                .plex_selected_server_token
                .clone()
                .filter(|token| !token.is_blank())
                .unwrap_or_else(|| token.clone()),
            owned: true,
            has_local_connection: plex_server_connection_kind_from_uri(uri)
                == PlexServerConnectionKind::Local,
            connection_kind: plex_server_connection_kind_from_uri(uri),
        };
        reachability.insert(
            plex_server_reachability_key(&selected_server.uri),
            verify_plex_server_reachability(&discovery, &selected_server),
        );
    }

    Ok(GuiPlexServerRefreshOutcome {
        servers,
        reachability,
    })
}

fn verify_plex_server_reachability<T>(
    discovery: &PlexDiscoveryService<T>,
    server: &PlexServerConnection,
) -> GuiPlexServerReachability
where
    T: sorotte_plex::discovery::PlexServerDiscoveryTransport,
{
    match discovery.verify(server) {
        Ok(()) => GuiPlexServerReachability::Reachable,
        Err(_error) => GuiPlexServerReachability::Unreachable,
    }
}

fn plex_server_reachability_key(uri: &str) -> String {
    uri.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn saved_plex_server_label(selected_server_id: Option<&str>) -> String {
    selected_server_id
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("Saved server {value}"))
        .unwrap_or_else(|| "Saved Plex server".to_owned())
}

fn plex_sync_state_label(state: PlexSyncState) -> &'static str {
    match state {
        PlexSyncState::Disconnected => "disconnected",
        PlexSyncState::Authenticating => "authenticating",
        PlexSyncState::Ready => "ready",
        PlexSyncState::Syncing => "syncing",
        PlexSyncState::Error => "error",
    }
}

fn format_plex_report_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix {seconds}")
}

fn open_system_url(url: &str) -> Result<(), String> {
    let mut command = open_system_url_command(url);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn open_system_url_command(url: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        command
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::support::test_temp_root;
    use crate::app::{GuiOwnedPlayer, SecretDraft};

    #[test]
    fn accepted_then_rejected_plex_load_never_enters_watch_sync() {
        #[derive(Default)]
        struct AcceptedPlexState {
            outcomes: std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
        }

        struct AcceptedPlexPlayer {
            state: std::sync::Arc<std::sync::Mutex<AcceptedPlexState>>,
        }

        impl PlayerAdapter for AcceptedPlexPlayer {
            fn name(&self) -> &'static str {
                "accepted-plex"
            }

            fn execute_tracked(
                &mut self,
                command: sorotte_player_api::PlayerCommand,
            ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError>
            {
                assert!(matches!(
                    command,
                    sorotte_player_api::PlayerCommand::OpenFile(_)
                ));
                Ok(sorotte_player_api::PlayerCommandId::new(17))
            }

            fn take_media_load_outcome(
                &mut self,
            ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .outcomes
                    .pop_front()
            }
        }

        let playlist_uri = sorotte_plex::PlexPlaylistUri {
            machine_identifier: "machine-1".to_owned(),
            rating_key: "rejected-episode".to_owned(),
            title: Some("Rejected Episode".to_owned()),
            file_name: Some("Rejected Episode.mkv".to_owned()),
            duration_millis: Some(90_000),
            size_bytes: Some(123_456),
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        };
        let logical_uri = sorotte_plex::format_plex_playlist_uri(&playlist_uri);
        let logical_file = LocalFileUpdate::new("Rejected Episode.mkv")
            .with_path(logical_uri.clone())
            .with_duration_seconds(90.0)
            .with_size_bytes(123_456);
        let playback_url =
            "http://127.0.0.1:32400/library/parts/rejected/file.mkv?X-Plex-Token=secret-token";
        let stream_target = sorotte_plex::PlexStreamTarget {
            playlist_uri,
            matched_item: sorotte_plex::PlexMatchedItem {
                rating_key: "rejected-episode".to_owned(),
                title: "Rejected Episode".to_owned(),
                media_type: sorotte_plex::PlexMediaType::Episode,
                duration_millis: Some(90_000),
            },
            logical_file: logical_file.clone(),
            playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(playback_url),
        };
        let player_state = std::sync::Arc::new(std::sync::Mutex::new(AcceptedPlexState::default()));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(AcceptedPlexPlayer {
            state: player_state.clone(),
        })));

        owner
            .open_plex_stream_target_through_attached_player_result_impl(
                &logical_uri,
                stream_target,
                false,
            )
            .expect("Plex stream open should have a player")
            .expect("Plex stream command should be accepted");

        assert_eq!(owner.player_local_file, Some(logical_file));
        assert!(owner.player_local_file_placeholder);
        assert!(
            owner.plex_watch_event_for_current_player().is_none(),
            "command acceptance must not create a Plex watch event"
        );

        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .outcomes
            .push_back(sorotte_player_api::PlayerMediaLoadOutcome::failure(
                playback_url,
                None,
                sorotte_player_api::PlayerMediaLoadFailureKind::Unknown,
                "player rejected the Plex stream",
            ));
        owner.refresh_player_state_impl();

        assert!(owner.player_local_file.is_none());
        assert!(!owner.player_local_file_placeholder);
        assert!(owner.pending_logical_media_override.is_none());
        assert!(
            owner.plex_watch_event_for_current_player().is_none(),
            "a rejected Plex load must never reach watch synchronization"
        );
    }

    #[test]
    fn open_system_url_command_preserves_plex_auth_url_as_one_argument() {
        let url = "https://app.plex.tv/auth#?clientID=sorotte&code=ABCD&context%5Bdevice%5D%5Bproduct%5D=Sorotte";
        let command = open_system_url_command(url);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        #[cfg(target_os = "windows")]
        {
            assert_eq!(command.get_program().to_string_lossy(), "rundll32");
            assert_eq!(args, vec!["url.dll,FileProtocolHandler", url]);
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(command.get_program().to_string_lossy(), "open");
            assert_eq!(args, vec![url]);
        }
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        {
            assert_eq!(command.get_program().to_string_lossy(), "xdg-open");
            assert_eq!(args, vec![url]);
        }
    }

    #[test]
    fn reconcile_plex_server_selection_updates_stale_duplicate_uri_by_machine_id() {
        let mut settings = StoredClientSettingsMvp {
            plex_selected_server_id: Some("raptor-machine".to_owned()),
            plex_selected_server_url: Some(
                "https://172-18-0-6.raptor-machine.plex.direct:32400".to_owned(),
            ),
            plex_selected_server_token: Some("old-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let servers = vec![PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: "https://125-209-152-187.raptor-machine.plex.direct:32400".to_owned(),
            access_token: "new-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        }];

        assert!(reconcile_plex_server_selection(
            &mut settings,
            &servers,
            true
        ));
        assert_eq!(
            settings.plex_selected_server_url.as_deref(),
            Some("https://125-209-152-187.raptor-machine.plex.direct:32400")
        );
        assert_eq!(
            settings
                .plex_selected_server_token
                .as_ref()
                .map(|token| token.expose_secret()),
            Some("new-token")
        );
    }

    #[test]
    fn reconcile_plex_server_selection_refreshes_token_for_same_server_identity() {
        let mut settings = StoredClientSettingsMvp {
            plex_selected_server_id: Some("raptor-machine".to_owned()),
            plex_selected_server_url: Some("https://raptor.example:32400".to_owned()),
            plex_selected_server_token: Some("old-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let servers = vec![PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: "https://raptor.example:32400".to_owned(),
            access_token: "new-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        }];

        assert!(reconcile_plex_server_selection(
            &mut settings,
            &servers,
            true
        ));
        assert_eq!(
            settings
                .plex_selected_server_token
                .as_ref()
                .map(|token| token.expose_secret()),
            Some("new-token")
        );
    }

    #[test]
    fn plex_snapshot_surfaces_saved_server_reachability_before_discovery_finishes() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.plex_server_reachability.insert(
            plex_server_reachability_key("https://raptor.example:32400"),
            GuiPlexServerReachability::Checking,
        );
        let settings = StoredClientSettingsMvp {
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("raptor-machine".to_owned()),
            plex_selected_server_url: Some("https://raptor.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };

        let snapshot = owner.plex_snapshot_from_settings_and_status(&settings, None);

        assert_eq!(snapshot.servers.len(), 1);
        assert!(snapshot.servers[0].selected);
        assert_eq!(
            snapshot.servers[0].reachability,
            GuiPlexServerReachability::Checking
        );
        assert_eq!(snapshot.servers[0].uri, "https://raptor.example:32400");
        assert_eq!(
            snapshot.servers[0].connection_kind,
            PlexServerConnectionKind::Remote
        );
    }

    #[test]
    fn plex_snapshot_keeps_saved_selected_server_when_discovery_returns_other_servers() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.plex_servers.push(PlexServerConnection {
            name: "Other Plex".to_owned(),
            machine_identifier: "other-machine".to_owned(),
            uri: "https://other.example:32400".to_owned(),
            access_token: "other-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        });
        owner.plex_server_reachability.insert(
            plex_server_reachability_key("https://saved.example:32400"),
            GuiPlexServerReachability::Reachable,
        );
        let settings = StoredClientSettingsMvp {
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("saved-machine".to_owned()),
            plex_selected_server_url: Some("https://saved.example:32400".to_owned()),
            plex_selected_server_token: Some("saved-token".into()),
            ..StoredClientSettingsMvp::default()
        };

        let snapshot = owner.plex_snapshot_from_settings_and_status(&settings, None);

        assert_eq!(snapshot.servers.len(), 2);
        let saved = snapshot
            .servers
            .iter()
            .find(|server| server.uri == "https://saved.example:32400")
            .expect("saved selected server should be present");
        assert!(saved.selected);
        assert_eq!(saved.reachability, GuiPlexServerReachability::Reachable);
    }

    #[test]
    fn manual_discovery_generation_supersedes_startup_without_accepting_its_result() {
        let mut coordinator = GuiPlexServerDiscoveryCoordinator::default();
        let startup_token = sorotte_secret::SecretValue::from("same-account-token");
        let operation_context = coordinator.operation_context(&StoredClientSettingsMvp {
            plex_user_token: Some(startup_token.clone()),
            ..StoredClientSettingsMvp::default()
        });
        let startup_generation = coordinator.begin_generation();
        let (startup_tx, startup_rx) = mpsc::channel();
        coordinator.install(
            startup_generation,
            operation_context.clone(),
            GuiPlexServerRefreshContext::Startup,
            startup_rx,
        );

        let manual_generation = coordinator.begin_generation();
        let (manual_tx, manual_rx) = mpsc::channel();
        coordinator.install(
            manual_generation,
            operation_context.clone(),
            GuiPlexServerRefreshContext::Manual,
            manual_rx,
        );

        assert!(
            startup_tx
                .send(GuiPlexServerDiscoveryWorkerResult {
                    generation: startup_generation,
                    operation_context: operation_context.clone(),
                    context: GuiPlexServerRefreshContext::Startup,
                    result: Ok(GuiPlexServerRefreshOutcome {
                        servers: Vec::new(),
                        reachability: HashMap::new(),
                    }),
                })
                .is_err(),
            "manual discovery must drop ownership of the startup receiver"
        );
        manual_tx
            .send(GuiPlexServerDiscoveryWorkerResult {
                generation: manual_generation,
                operation_context: operation_context.clone(),
                context: GuiPlexServerRefreshContext::Manual,
                result: Ok(GuiPlexServerRefreshOutcome {
                    servers: Vec::new(),
                    reachability: HashMap::new(),
                }),
            })
            .expect("current manual result should retain a receiver");
        let manual_job = coordinator
            .take_active()
            .expect("manual job should remain active");
        let current = manual_job
            .receiver
            .recv()
            .expect("manual result should arrive");
        assert!(coordinator.accepts(&manual_job, &current, &operation_context));
    }

    #[test]
    fn login_token_generation_invalidates_old_discovery_results_without_storing_raw_tokens() {
        const OLD_TOKEN: &str = "OLD_DISCOVERY_TOKEN_CANARY";
        const LOGIN_TOKEN: &str = "LOGIN_DISCOVERY_TOKEN_CANARY";
        let mut coordinator = GuiPlexServerDiscoveryCoordinator::default();
        let old_operation_context = coordinator.operation_context(&StoredClientSettingsMvp {
            plex_user_token: Some(OLD_TOKEN.into()),
            ..StoredClientSettingsMvp::default()
        });
        coordinator.invalidate_operation_context();
        let login_operation_context = coordinator.operation_context(&StoredClientSettingsMvp {
            plex_user_token: Some(LOGIN_TOKEN.into()),
            ..StoredClientSettingsMvp::default()
        });
        let old_generation = coordinator.begin_generation();
        let login_generation = coordinator.begin_generation();
        let stale = GuiPlexServerDiscoveryWorkerResult {
            generation: old_generation,
            operation_context: old_operation_context,
            context: GuiPlexServerRefreshContext::Manual,
            result: Ok(GuiPlexServerRefreshOutcome {
                servers: Vec::new(),
                reachability: HashMap::new(),
            }),
        };
        let current = GuiPlexServerDiscoveryWorkerResult {
            generation: login_generation,
            operation_context: login_operation_context.clone(),
            context: GuiPlexServerRefreshContext::Login,
            result: Ok(GuiPlexServerRefreshOutcome {
                servers: Vec::new(),
                reachability: HashMap::new(),
            }),
        };
        let wrong_token_context = coordinator.operation_context(&StoredClientSettingsMvp {
            plex_user_token: Some("WRONG_DISCOVERY_TOKEN_CANARY".into()),
            ..StoredClientSettingsMvp::default()
        });
        let wrong_token = GuiPlexServerDiscoveryWorkerResult {
            generation: login_generation,
            operation_context: wrong_token_context,
            context: GuiPlexServerRefreshContext::Login,
            result: Ok(GuiPlexServerRefreshOutcome {
                servers: Vec::new(),
                reachability: HashMap::new(),
            }),
        };
        let wrong_context = GuiPlexServerDiscoveryWorkerResult {
            generation: login_generation,
            operation_context: login_operation_context.clone(),
            context: GuiPlexServerRefreshContext::Manual,
            result: Ok(GuiPlexServerRefreshOutcome {
                servers: Vec::new(),
                reachability: HashMap::new(),
            }),
        };
        let (_login_tx, login_rx) = mpsc::channel();
        let login_job = GuiPlexServerDiscoveryJob {
            generation: login_generation,
            operation_context: login_operation_context.clone(),
            context: GuiPlexServerRefreshContext::Login,
            receiver: login_rx,
        };

        assert!(!coordinator.accepts(&login_job, &stale, &login_operation_context));
        assert!(!coordinator.accepts(&login_job, &wrong_token, &login_operation_context));
        assert!(!coordinator.accepts(&login_job, &wrong_context, &login_operation_context));
        assert!(coordinator.accepts(&login_job, &current, &login_operation_context));
        let debug = format!("{stale:?} {current:?} {wrong_token:?}");
        assert!(!debug.contains(OLD_TOKEN));
        assert!(!debug.contains(LOGIN_TOKEN));
        assert!(!debug.contains("WRONG_DISCOVERY_TOKEN_CANARY"));
        assert!(debug.contains("authenticated"));
    }

    fn test_plex_search_result(rating_key: &str, title: &str) -> GuiPlexPlaylistSearchResult {
        GuiPlexPlaylistSearchResult {
            rating_key: rating_key.to_owned(),
            title: title.to_owned(),
            parent_title: None,
            grandparent_title: None,
            media_type: sorotte_plex::PlexMediaType::Movie,
            duration_millis: Some(90_000),
            file_name: Some(format!("{title}.mkv")),
        }
    }

    fn mark_plex_picker_busy(
        state: &mut SorotteGuiShellAppState,
        query: &str,
        adding_rating_key: Option<&str>,
    ) {
        state.plex_playlist_search = Some(Default::default());
        let picker = state
            .plex_playlist_search
            .as_mut()
            .expect("test Plex picker should exist");
        picker.query = query.to_owned();
        picker.searching = true;
        picker.adding_rating_key = adding_rating_key.map(str::to_owned);
    }

    fn active_plex_search_job(
        id: u64,
        operation_context: GuiPlexOperationContext,
        query: &str,
        result_rx: mpsc::Receiver<GuiPlexPlaylistSearchWorkerResult>,
    ) -> GuiActivePlexPlaylistSearchJob {
        GuiActivePlexPlaylistSearchJob {
            id,
            operation_context,
            query: query.to_owned(),
            result_rx,
        }
    }

    fn active_plex_resolve_job(
        id: u64,
        operation_context: GuiPlexOperationContext,
        rating_key: &str,
        result_rx: mpsc::Receiver<GuiPlexPlaylistResolveWorkerResult>,
    ) -> GuiActivePlexPlaylistResolveJob {
        GuiActivePlexPlaylistResolveJob {
            id,
            operation_context,
            rating_key: rating_key.to_owned(),
            result_rx,
        }
    }

    fn install_blocked_plex_playlist_jobs(
        owner: &mut GuiPersistedConfigRuntimeOwner,
        state: &mut SorotteGuiShellAppState,
        query: &str,
        rating_key: &str,
    ) -> (
        mpsc::Sender<GuiPlexPlaylistSearchWorkerResult>,
        mpsc::Sender<GuiPlexPlaylistResolveWorkerResult>,
    ) {
        mark_plex_picker_busy(state, query, Some(rating_key));
        let operation_context =
            owner.plex_operation_context(&state.configuration.to_stored_settings());
        let search_id = owner.begin_plex_playlist_job_generation();
        let resolve_id = owner.begin_plex_playlist_job_generation();
        let (search_tx, search_rx) = mpsc::channel();
        let (resolve_tx, resolve_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            search_id,
            operation_context.clone(),
            query,
            search_rx,
        ));
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            resolve_id,
            operation_context,
            rating_key,
            resolve_rx,
        ));
        (search_tx, resolve_tx)
    }

    fn assert_plex_playlist_jobs_cancelled(
        owner: &GuiPersistedConfigRuntimeOwner,
        state: &SorotteGuiShellAppState,
    ) {
        assert!(owner.plex_playlist_search_job.is_none());
        assert!(owner.plex_playlist_resolve_job.is_none());
        assert!(state.plex_playlist_search.is_none());
    }

    #[test]
    fn cancelled_resolve_has_no_playlist_side_effect() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        mark_plex_picker_busy(&mut state, "movie", Some("rating-a"));
        let operation_context = owner.plex_operation_context(&settings);
        let id = owner.begin_plex_playlist_job_generation();
        let (resolve_tx, resolve_rx) = mpsc::channel();
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            id,
            operation_context.clone(),
            "rating-a",
            resolve_rx,
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let playlist_before = state.current_shared_playlist_entries().to_vec();

        assert!(state.apply(GuiShellAction::CancelPlexPlaylistSearch));
        assert!(owner.handle_runtime_request(
            &handle,
            &mut state,
            GuiRuntimeRequest::CancelPlexPlaylistJobs {
                reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
            },
        ));

        assert!(
            resolve_tx
                .send(GuiPlexPlaylistResolveWorkerResult {
                    id,
                    operation_context,
                    rating_key: "rating-a".to_owned(),
                    result: Ok("plex://machine/library/metadata/rating-a".to_owned()),
                })
                .is_err(),
            "cancellation must release the resolve receiver"
        );
        owner.pump_plex_playlist_workers(&handle, &mut state);

        assert!(owner.plex_playlist_resolve_job.is_none());
        assert!(state.plex_playlist_search.is_none());
        assert_eq!(state.current_shared_playlist_entries(), playlist_before);
        assert!(handle.drain_requests().is_empty());
        assert!(handle.drain_actions().is_empty());
    }

    #[test]
    fn queued_cancel_is_ordered_before_ready_resolve_commit() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        mark_plex_picker_busy(&mut state, "movie", Some("rating-a"));
        let operation_context = owner.plex_operation_context(&settings);
        let id = owner.begin_plex_playlist_job_generation();
        let (resolve_tx, resolve_rx) = mpsc::channel();
        resolve_tx
            .send(GuiPlexPlaylistResolveWorkerResult {
                id,
                operation_context: operation_context.clone(),
                rating_key: "rating-a".to_owned(),
                result: Ok("plex://machine/library/metadata/rating-a".to_owned()),
            })
            .expect("ready resolve result should queue");
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            id,
            operation_context,
            "rating-a",
            resolve_rx,
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let playlist_before = state.current_shared_playlist_entries().to_vec();
        handle.push_request(GuiRuntimeRequest::CancelPlexPlaylistJobs {
            reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
        });

        owner.drain_plex_playlist_resolve_worker(&handle, &mut state);

        assert!(
            owner.plex_playlist_resolve_job.is_some(),
            "a pending command must defer without consuming the ready result"
        );
        assert_eq!(state.current_shared_playlist_entries(), playlist_before);
        assert!(handle.drain_actions().is_empty());
        let requests = handle.drain_requests();
        assert_eq!(requests.len(), 1);
        assert!(state.apply(GuiShellAction::CancelPlexPlaylistSearch));
        for request in requests {
            assert!(owner.handle_runtime_request(&handle, &mut state, request));
        }

        assert!(owner.plex_playlist_resolve_job.is_none());
        owner.drain_plex_playlist_resolve_worker(&handle, &mut state);
        assert_eq!(state.current_shared_playlist_entries(), playlist_before);
        assert!(handle.drain_requests().is_empty());
        assert!(handle.drain_actions().is_empty());
    }

    #[test]
    fn fast_search_result_uses_request_intent_before_shell_input_arrives() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut stale_projection = SorotteGuiShellAppState::from_stored_settings(&settings);
        assert!(stale_projection.plex_playlist_search.is_none());
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_search_selected_plex_server_media_request(
            &handle,
            &mut stale_projection,
            "movie".to_owned(),
        ));
        let (id, operation_context) = owner
            .plex_playlist_search_job
            .as_ref()
            .map(|job| (job.id, job.operation_context.clone()))
            .expect("search request should install a job");
        let (result_tx, result_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            id,
            operation_context.clone(),
            "movie",
            result_rx,
        ));
        result_tx
            .send(GuiPlexPlaylistSearchWorkerResult {
                id,
                operation_context,
                query: "movie".to_owned(),
                result: Ok(vec![test_plex_search_result("current", "Current Movie")]),
            })
            .expect("fast current result should queue");

        owner.drain_plex_playlist_search_worker(&handle, &mut stale_projection);

        let picker = stale_projection
            .plex_playlist_search
            .as_ref()
            .expect("request intent should create the runtime picker projection");
        assert!(!picker.searching);
        assert_eq!(picker.query, "movie");
        assert_eq!(picker.results[0].rating_key, "current");
        assert!(owner.plex_playlist_search_job.is_none());
    }

    #[test]
    fn fast_resolve_result_uses_request_intent_before_shell_input_arrives() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut stale_projection = SorotteGuiShellAppState::from_stored_settings(&settings);
        assert!(stale_projection.plex_playlist_search.is_none());
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_resolve_plex_playlist_item_request(
            &handle,
            &mut stale_projection,
            "rating-a".to_owned(),
        ));
        let (id, operation_context) = owner
            .plex_playlist_resolve_job
            .as_ref()
            .map(|job| (job.id, job.operation_context.clone()))
            .expect("resolve request should install a job");
        let (result_tx, result_rx) = mpsc::channel();
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            id,
            operation_context.clone(),
            "rating-a",
            result_rx,
        ));
        result_tx
            .send(GuiPlexPlaylistResolveWorkerResult {
                id,
                operation_context,
                rating_key: "rating-a".to_owned(),
                result: Ok("plex://machine/library/metadata/rating-a".to_owned()),
            })
            .expect("fast current result should queue");

        owner.drain_plex_playlist_resolve_worker(&handle, &mut stale_projection);

        assert_eq!(
            stale_projection
                .current_shared_playlist_entries()
                .last()
                .map(String::as_str),
            Some("plex://machine/library/metadata/rating-a")
        );
        assert!(owner.plex_playlist_resolve_job.is_none());
        assert_eq!(handle.drain_requests().len(), 1);
    }

    #[test]
    fn cancelled_and_reopened_search_rejects_old_same_context_result() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        mark_plex_picker_busy(&mut state, "search-a", None);
        let operation_context = owner.plex_operation_context(&settings);
        let old_id = owner.begin_plex_playlist_job_generation();
        let (old_tx, old_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            old_id,
            operation_context.clone(),
            "search-a",
            old_rx,
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(state.apply(GuiShellAction::CancelPlexPlaylistSearch));
        assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
        assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
            query: "search-b".to_owned(),
        }));
        assert!(owner.handle_runtime_request(
            &handle,
            &mut state,
            GuiRuntimeRequest::CancelPlexPlaylistJobs {
                reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
            },
        ));

        let reopened = state
            .plex_playlist_search
            .as_ref()
            .expect("a delayed picker-close cancellation must not close the reopened picker");
        assert_eq!(reopened.query, "search-b");
        assert!(reopened.searching);
        assert!(
            old_tx
                .send(GuiPlexPlaylistSearchWorkerResult {
                    id: old_id,
                    operation_context: operation_context.clone(),
                    query: "search-a".to_owned(),
                    result: Ok(vec![test_plex_search_result("a", "Search A")]),
                })
                .is_err(),
            "cancelled search A must no longer own a receiver"
        );

        let new_id = owner.begin_plex_playlist_job_generation();
        let (new_tx, new_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            new_id,
            operation_context.clone(),
            "search-b",
            new_rx,
        ));
        owner.drain_plex_playlist_search_worker(&handle, &mut state);
        assert_eq!(
            owner
                .plex_playlist_search_job
                .as_ref()
                .map(|job| (job.id, job.query.as_str())),
            Some((new_id, "search-b"))
        );

        new_tx
            .send(GuiPlexPlaylistSearchWorkerResult {
                id: new_id,
                operation_context,
                query: "search-b".to_owned(),
                result: Ok(vec![test_plex_search_result("b", "Search B")]),
            })
            .expect("current search B result should queue");
        owner.drain_plex_playlist_search_worker(&handle, &mut state);

        let completed = state
            .plex_playlist_search
            .as_ref()
            .expect("search B picker should remain open");
        assert_eq!(completed.query, "search-b");
        assert!(!completed.searching);
        assert_eq!(
            completed
                .results
                .iter()
                .map(|result| result.rating_key.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    #[test]
    fn same_context_resolve_supersession_only_appends_latest_item() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        mark_plex_picker_busy(&mut state, "movie", Some("rating-a"));
        let operation_context = owner.plex_operation_context(&settings);
        let old_id = owner.begin_plex_playlist_job_generation();
        let (old_tx, old_rx) = mpsc::channel();
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            old_id,
            operation_context,
            "rating-a",
            old_rx,
        ));
        state
            .plex_playlist_search
            .as_mut()
            .expect("Plex picker should exist")
            .adding_rating_key = Some("rating-b".to_owned());
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_resolve_plex_playlist_item_request(
            &handle,
            &mut state,
            "rating-b".to_owned(),
        ));
        assert!(
            old_tx
                .send(GuiPlexPlaylistResolveWorkerResult {
                    id: old_id,
                    operation_context: owner.plex_operation_context(&settings),
                    rating_key: "rating-a".to_owned(),
                    result: Ok("plex://machine/library/metadata/rating-a".to_owned()),
                })
                .is_err(),
            "resolve B must supersede resolve A immediately"
        );
        let (new_id, new_context) = owner
            .plex_playlist_resolve_job
            .as_ref()
            .map(|job| (job.id, job.operation_context.clone()))
            .expect("resolve B should own an active job");
        assert_ne!(new_id, old_id);
        let (new_tx, new_rx) = mpsc::channel();
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            new_id,
            new_context.clone(),
            "rating-b",
            new_rx,
        ));
        new_tx
            .send(GuiPlexPlaylistResolveWorkerResult {
                id: new_id,
                operation_context: new_context,
                rating_key: "rating-b".to_owned(),
                result: Ok("plex://machine/library/metadata/rating-b".to_owned()),
            })
            .expect("resolve B result should queue");

        owner.drain_plex_playlist_resolve_worker(&handle, &mut state);

        assert_eq!(
            handle.drain_requests(),
            vec![GuiRuntimeRequest::QueuePlaylistEntry {
                entry: "plex://machine/library/metadata/rating-b".to_owned(),
                select_after_queue: false,
            }]
        );
        assert_eq!(
            state
                .current_shared_playlist_entries()
                .last()
                .map(String::as_str),
            Some("plex://machine/library/metadata/rating-b")
        );
        assert!(
            !state
                .current_shared_playlist_entries()
                .iter()
                .any(|entry| entry.contains("rating-a"))
        );
    }

    #[test]
    fn busy_search_request_supersedes_active_same_context_job() {
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        mark_plex_picker_busy(&mut state, "search-b", None);
        let operation_context = owner.plex_operation_context(&settings);
        let old_id = owner.begin_plex_playlist_job_generation();
        let (old_tx, old_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            old_id,
            operation_context.clone(),
            "search-a",
            old_rx,
        ));
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_search_selected_plex_server_media_request(
            &handle,
            &mut state,
            "search-b".to_owned(),
        ));

        let current = owner
            .plex_playlist_search_job
            .as_ref()
            .expect("busy search request must install replacement work");
        assert_ne!(current.id, old_id);
        assert_eq!(current.operation_context, operation_context);
        assert_eq!(current.query, "search-b");
        assert!(
            old_tx
                .send(GuiPlexPlaylistSearchWorkerResult {
                    id: old_id,
                    operation_context,
                    query: "search-a".to_owned(),
                    result: Ok(Vec::new()),
                })
                .is_err(),
            "replacement search must drop the previous receiver"
        );
    }

    #[test]
    fn auth_completion_clears_old_account_server_identity_before_discovery() {
        let mut settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_sync_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_user_token: Some("old-account-token".into()),
            plex_selected_server_id: Some("old-machine".to_owned()),
            plex_selected_server_url: Some("https://old.example:32400".to_owned()),
            plex_selected_server_token: Some("old-server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let previous_settings = settings.clone();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.plex_servers.push(PlexServerConnection {
            name: "Old Server".to_owned(),
            machine_identifier: "old-machine".to_owned(),
            uri: "https://old.example:32400".to_owned(),
            access_token: "old-server-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        });
        owner.plex_server_reachability.insert(
            plex_server_reachability_key("https://old.example:32400"),
            GuiPlexServerReachability::Reachable,
        );
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let previous_context = owner.plex_operation_context(&settings);
        let (_search_tx, _resolve_tx) =
            install_blocked_plex_playlist_jobs(&mut owner, &mut state, "old-query", "old-rating");

        owner.apply_authenticated_plex_account(&mut settings, "new-account-token".into());
        owner.invalidate_plex_operation_context_if_settings_changed(
            &handle,
            &mut state,
            &previous_settings,
            &settings,
        );

        assert_eq!(
            settings.plex_user_token.as_ref(),
            Some(&sorotte_secret::SecretValue::from("new-account-token"))
        );
        assert!(settings.plex_selected_server_id.is_none());
        assert!(settings.plex_selected_server_url.is_none());
        assert!(settings.plex_selected_server_token.is_none());
        assert!(settings.plex_sync_enabled == Some(true));
        assert!(settings.plex_streaming_enabled == Some(true));
        assert!(owner.plex_servers.is_empty());
        assert!(owner.plex_server_reachability.is_empty());
        assert_plex_playlist_jobs_cancelled(&owner, &state);
        assert!(!plex_config_from_settings(&settings).has_selected_server());
        assert_ne!(owner.plex_operation_context(&settings), previous_context);
    }

    #[test]
    fn new_account_search_completion_wins_before_old_account_worker_finishes() {
        let old_settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_user_token: Some("old-account-token".into()),
            plex_selected_server_id: Some("old-machine".to_owned()),
            plex_selected_server_url: Some("https://old.example:32400".to_owned()),
            plex_selected_server_token: Some("old-server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let new_settings = StoredClientSettingsMvp {
            plex_user_token: Some("new-account-token".into()),
            plex_selected_server_id: Some("new-machine".to_owned()),
            plex_selected_server_url: Some("https://new.example:32400".to_owned()),
            plex_selected_server_token: Some("new-server-token".into()),
            ..old_settings.clone()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let old_context = owner.plex_operation_context(&old_settings);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&old_settings);
        mark_plex_picker_busy(&mut state, "movie", None);
        let (old_tx, old_rx) = mpsc::channel();
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            1,
            old_context.clone(),
            "movie",
            old_rx,
        ));

        owner.invalidate_plex_operation_context_if_settings_changed(
            &handle,
            &mut state,
            &old_settings,
            &new_settings,
        );
        assert!(state.plex_playlist_search.is_none());
        state = SorotteGuiShellAppState::from_stored_settings(&new_settings);
        assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
        assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
            query: "movie".to_owned(),
        }));
        let new_context = owner.plex_operation_context(&new_settings);
        let (new_tx, new_rx) = mpsc::channel();
        new_tx
            .send(GuiPlexPlaylistSearchWorkerResult {
                id: 2,
                operation_context: new_context.clone(),
                query: "movie".to_owned(),
                result: Ok(vec![test_plex_search_result("new", "New Account Movie")]),
            })
            .expect("new account search result should queue");
        owner.plex_playlist_search_job =
            Some(active_plex_search_job(2, new_context, "movie", new_rx));

        owner.drain_plex_playlist_search_worker(&handle, &mut state);

        assert_eq!(
            state
                .plex_playlist_search
                .as_ref()
                .and_then(|search| search.results.first())
                .map(|result| result.rating_key.as_str()),
            Some("new")
        );
        assert!(
            old_tx
                .send(GuiPlexPlaylistSearchWorkerResult {
                    id: 1,
                    operation_context: old_context,
                    query: "movie".to_owned(),
                    result: Ok(vec![test_plex_search_result("old", "Old Account Movie")]),
                })
                .is_err(),
            "reauthentication must release the old account search receiver"
        );
    }

    #[test]
    fn stale_server_resolve_cannot_append_after_new_server_result_completed_first() {
        let old_settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("old-machine".to_owned()),
            plex_selected_server_url: Some("https://old.example:32400".to_owned()),
            plex_selected_server_token: Some("old-server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let new_settings = StoredClientSettingsMvp {
            plex_selected_server_id: Some("new-machine".to_owned()),
            plex_selected_server_url: Some("https://new.example:32400".to_owned()),
            plex_selected_server_token: Some("new-server-token".into()),
            ..old_settings.clone()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let old_context = owner.plex_operation_context(&old_settings);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&old_settings);
        owner.invalidate_plex_operation_context_if_settings_changed(
            &handle,
            &mut state,
            &old_settings,
            &new_settings,
        );
        let new_context = owner.plex_operation_context(&new_settings);
        state = SorotteGuiShellAppState::from_stored_settings(&new_settings);
        mark_plex_picker_busy(&mut state, "movie", Some("new-rating"));

        let (new_tx, new_rx) = mpsc::channel();
        new_tx
            .send(GuiPlexPlaylistResolveWorkerResult {
                id: 2,
                operation_context: new_context.clone(),
                rating_key: "new-rating".to_owned(),
                result: Ok("plex://new-machine/library/metadata/new-rating".to_owned()),
            })
            .expect("new server resolve result should queue");
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            2,
            new_context,
            "new-rating",
            new_rx,
        ));
        owner.drain_plex_playlist_resolve_worker(&handle, &mut state);
        assert_eq!(handle.drain_requests().len(), 1);

        state
            .plex_playlist_search
            .as_mut()
            .expect("Plex picker should remain open")
            .adding_rating_key = Some("old-rating".to_owned());

        let (old_tx, old_rx) = mpsc::channel();
        old_tx
            .send(GuiPlexPlaylistResolveWorkerResult {
                id: 1,
                operation_context: old_context,
                rating_key: "old-rating".to_owned(),
                result: Ok("plex://old-machine/library/metadata/old-rating".to_owned()),
            })
            .expect("stale server resolve result should queue for validation");
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            1,
            owner.plex_operation_context(&old_settings),
            "old-rating",
            old_rx,
        ));
        owner.drain_plex_playlist_resolve_worker(&handle, &mut state);

        assert!(handle.drain_requests().is_empty());
        assert_eq!(
            state
                .current_shared_playlist_entries()
                .last()
                .map(String::as_str),
            Some("plex://new-machine/library/metadata/new-rating")
        );
        assert!(
            !state
                .current_shared_playlist_entries()
                .iter()
                .any(|entry| entry.contains("old-rating"))
        );
    }

    #[test]
    fn sync_toggle_cancels_active_playlist_jobs_and_closes_picker() {
        let root = test_temp_root("plex-sync-toggle-cancels-playlist-jobs");
        let config_path = root.join("sorotte.ini");
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_sync_enabled: Some(false),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let (_search_tx, _resolve_tx) =
            install_blocked_plex_playlist_jobs(&mut owner, &mut state, "movie", "rating-a");
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_toggle_plex_sync_request(&handle, &mut state, true));

        assert_plex_playlist_jobs_cancelled(&owner, &state);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_sync_tick_cannot_reinstall_engine_after_plex_disconnect() {
        let test_root = std::env::temp_dir().join(format!(
            "sorotte-gui-stale-plex-sync-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&test_root);
        let old_settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_sync_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(test_root.join("sorotte.ini")));
        let cache_path = owner
            .plex_cache_path()
            .expect("configured owner should provide a Plex cache path");
        let old_context = owner.plex_operation_context(&old_settings);
        let engine = owner
            .take_plex_sync_engine(plex_config_from_settings(&old_settings))
            .expect("test sync engine should be created");
        let mut state = SorotteGuiShellAppState::from_stored_settings(&old_settings);
        let (_search_tx, _resolve_tx) =
            install_blocked_plex_playlist_jobs(&mut owner, &mut state, "movie", "rating-a");
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_disconnect_plex_request(&handle, &mut state));
        assert_plex_playlist_jobs_cancelled(&owner, &state);
        let _ = handle.drain_actions();
        let staged_cache_write = engine
            .cache()
            .stage_to_path(&cache_path)
            .expect("stale sync cache should stage for acceptance testing");
        let (old_tx, old_rx) = mpsc::channel();
        old_tx
            .send(GuiPlexSyncWorkerResult {
                operation_context: old_context,
                engine,
                status: PlexSyncStatus::ready(),
                staged_cache_write: Some(Ok(staged_cache_write)),
            })
            .expect("stale sync result should queue for validation");
        owner.plex_sync_rx = Some(old_rx);

        owner.drain_plex_sync_worker(&handle, &mut state);

        assert!(owner.plex_sync_engine.is_none());
        assert!(!cache_path.exists());
        assert!(handle.drain_actions().is_empty());
        let _ = std::fs::remove_dir_all(test_root);
    }

    #[test]
    fn stale_stream_resolution_is_discarded_after_streaming_is_disabled() {
        let test_root = std::env::temp_dir().join(format!(
            "sorotte-gui-stale-plex-stream-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&test_root);
        let settings = StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine".to_owned()),
            plex_selected_server_url: Some("https://plex.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(test_root.join("sorotte.ini")));
        let cache_path = owner
            .plex_cache_path()
            .expect("configured owner should provide a Plex cache path");
        let old_context = owner.plex_operation_context(&settings);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let (_search_tx, _resolve_tx) =
            install_blocked_plex_playlist_jobs(&mut owner, &mut state, "movie", "rating-a");
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.handle_toggle_plex_streaming_request(&handle, &mut state, false));
        assert_plex_playlist_jobs_cancelled(&owner, &state);
        let _ = handle.drain_actions();
        let staged_cache_write = PlexMatchCache::default()
            .stage_to_path(&cache_path)
            .expect("stale stream cache should stage for acceptance testing");
        let (old_tx, old_rx) = mpsc::channel();
        old_tx
            .send(GuiPlexStreamResolveWorkerResult {
                operation_context: old_context.clone(),
                trigger_key: "old-stream-trigger".to_owned(),
                result: Ok(GuiPlexStreamResolveOutcome {
                    stream_target: Ok(None),
                    cache: PlexMatchCache::default(),
                }),
                staged_cache_write: Some(Ok(staged_cache_write)),
            })
            .expect("stale stream result should queue for validation");
        owner.plex_stream_resolve_rx = Some(old_rx);
        owner.plex_stream_resolve_trigger_key = Some("old-stream-trigger".to_owned());
        owner.plex_stream_resolve_context = Some(old_context);

        assert!(!owner.pump_plex_stream_resolution_worker(&state));
        assert!(owner.plex_stream_resolve_result.is_none());
        assert!(owner.plex_stream_resolve_trigger_key.is_none());
        assert!(owner.plex_stream_resolve_context.is_none());
        assert!(!cache_path.exists());
        let _ = std::fs::remove_dir_all(test_root);
    }

    #[test]
    fn selecting_plex_server_clears_stale_server_scoped_workers() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.plex_servers.push(PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: "https://raptor.example:32400".to_owned(),
            access_token: "server-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        });
        let (_sync_tx, sync_rx) = mpsc::channel();
        let (_search_tx, search_rx) = mpsc::channel();
        let (_resolve_tx, resolve_rx) = mpsc::channel();
        let (_stream_tx, stream_rx) = mpsc::channel();
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            plex_plugin_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            ..StoredClientSettingsMvp::default()
        });
        mark_plex_picker_busy(&mut state, "old-query", Some("old-rating"));
        let old_context = owner.plex_operation_context(&state.configuration.to_stored_settings());
        owner.plex_sync_rx = Some(sync_rx);
        owner.plex_sync_next_tick_due_at = Some(Instant::now());
        owner.plex_playlist_search_job = Some(active_plex_search_job(
            1,
            old_context.clone(),
            "old-query",
            search_rx,
        ));
        owner.plex_playlist_resolve_job = Some(active_plex_resolve_job(
            2,
            old_context,
            "old-rating",
            resolve_rx,
        ));
        owner.plex_stream_resolve_rx = Some(stream_rx);
        owner.plex_stream_resolve_trigger_key = Some("old-server-target".to_owned());

        assert!(owner.handle_select_plex_server_request(
            &handle,
            &mut state,
            "raptor-machine".to_owned(),
            "https://raptor.example:32400".to_owned(),
        ));

        assert!(owner.plex_sync_rx.is_none());
        assert!(owner.plex_sync_next_tick_due_at.is_none());
        assert!(owner.plex_playlist_search_job.is_none());
        assert!(owner.plex_playlist_resolve_job.is_none());
        assert!(state.plex_playlist_search.is_none());
        assert!(owner.plex_stream_resolve_rx.is_none());
        assert!(owner.plex_stream_resolve_trigger_key.is_none());
        assert_eq!(
            state
                .configuration
                .settings
                .plex_selected_server_url
                .as_deref(),
            Some("https://raptor.example:32400")
        );
    }

    #[test]
    fn completed_plex_login_persists_token_across_all_runtime_layers() {
        let root = test_temp_root("plex-login-four-layer-success");
        let config_path = root.join("sorotte.ini");
        let saved_settings = StoredClientSettingsMvp {
            host: Some("saved.example".to_owned()),
            language: Some("en".to_owned()),
            server_password: Some("saved-secret".into()),
            plex_plugin_enabled: Some(true),
            plex_sync_enabled: Some(false),
            plex_streaming_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        sorotte_client_app::app_boundary::persistence::upsert_sorotte_ini_stored_client_settings_mvp_at_path(
            &config_path,
            &saved_settings,
        )
        .expect("initial Plex login settings should persist");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::ConnectionHost,
            value: "draft.example".to_owned().into(),
        }));
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::GeneralLanguage,
            value: "fr".to_owned().into(),
        }));
        assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::ConnectionServerPassword,
            value: "replacement-secret".to_owned().into(),
        }));
        assert!(state.apply(GuiShellAction::FocusConfigurationControl(
            SettingId::GeneralLanguage,
        )));
        assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
            SettingId::GeneralLanguage,
        )));
        assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
            "de".to_owned().into(),
        )));
        let focused_before = state.focused_configuration_control.clone();
        let edit_before = state.text_edit_session.clone();

        owner.apply_plex_auth_poll_result(
            &handle,
            &mut state,
            true,
            PlexAuthPollResult {
                auth_token: Some("new-account-token".into()),
                expires_at: None,
            },
        );

        let disk = sorotte_client_app::app_boundary::persistence::load_sorotte_ini_stored_client_settings_mvp_from_path(
            &config_path,
        )
        .expect("Plex login config should be readable")
        .expect("Plex login config should exist");
        for (layer, settings) in [
            ("disk", &disk),
            ("saved", &state.saved_configuration),
            ("draft", &state.configuration.settings),
        ] {
            assert_eq!(
                settings
                    .plex_user_token
                    .as_ref()
                    .map(|token| token.expose_secret()),
                Some("new-account-token"),
                "successful login must install the token at the {layer} layer",
            );
        }
        for (layer, settings) in [("disk", &disk), ("saved", &state.saved_configuration)] {
            assert_eq!(settings.host.as_deref(), Some("saved.example"), "{layer}");
            assert_eq!(settings.language.as_deref(), Some("en"), "{layer}");
            assert_eq!(
                settings
                    .server_password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("saved-secret"),
                "{layer}",
            );
        }
        assert_eq!(
            state.configuration.settings.host.as_deref(),
            Some("draft.example")
        );
        assert_eq!(state.configuration.settings.language.as_deref(), Some("fr"));
        assert_eq!(
            state
                .configuration
                .settings
                .server_password
                .as_ref()
                .map(|password| password.expose_secret()),
            Some("saved-secret"),
            "the raw draft must retain the original secret baseline"
        );
        assert!(matches!(
            state.configuration.server_password,
            SecretDraft::Replace(ref password) if password.expose_secret() == "replacement-secret"
        ));
        assert_eq!(state.focused_configuration_control, focused_before);
        assert_eq!(state.text_edit_session, edit_before);
        assert!(state.has_unsaved_configuration_changes());
        assert!(state.plex.authenticated);
        assert!(owner.plex_runtime_snapshot.authenticated);
        assert_eq!(state.plex.status, "ready");
        assert!(handle.drain_actions().iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message,
            } if message == "Plex login complete."
        )));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_plex_login_that_cannot_be_saved_is_recoverable_and_does_not_commit() {
        let root = test_temp_root("plex-login-not-saved");
        std::fs::create_dir_all(&root).expect("test directory should be created");
        let sentinel_path = root.join("unchanged.txt");
        std::fs::write(&sentinel_path, "unchanged").expect("sentinel should be written");
        let saved_settings = StoredClientSettingsMvp {
            language: Some("en".to_owned()),
            server_password: Some("saved-secret".into()),
            plex_plugin_enabled: Some(true),
            plex_sync_enabled: Some(true),
            plex_user_token: Some("old-account-token".into()),
            plex_selected_server_id: Some("old-machine".to_owned()),
            plex_selected_server_url: Some("https://old.example:32400".to_owned()),
            plex_selected_server_token: Some("old-server-token".into()),
            ..StoredClientSettingsMvp::default()
        };
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.clone()));
        owner.plex_servers.push(PlexServerConnection {
            name: "Old Server".to_owned(),
            machine_identifier: "old-machine".to_owned(),
            uri: "https://old.example:32400".to_owned(),
            access_token: "old-server-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        });
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        assert!(state.apply(GuiShellAction::EditConfigurationText {
            id: SettingId::GeneralLanguage,
            value: "fr".to_owned().into(),
        }));
        assert!(state.apply(GuiShellAction::RemoveServerPassword));
        assert!(state.apply(GuiShellAction::FocusConfigurationControl(
            SettingId::GeneralLanguage,
        )));
        assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
            SettingId::GeneralLanguage,
        )));
        assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
            "de".to_owned().into(),
        )));
        let saved_before = state.saved_configuration.clone();
        let draft_before = state.configuration.settings.clone();
        let focused_before = state.focused_configuration_control.clone();
        let edit_before = state.text_edit_session.clone();

        owner.apply_plex_auth_poll_result(
            &handle,
            &mut state,
            true,
            PlexAuthPollResult {
                auth_token: Some("new-account-token".into()),
                expires_at: None,
            },
        );

        assert_eq!(state.saved_configuration, saved_before);
        assert_eq!(state.configuration.settings, draft_before);
        assert_eq!(state.configuration.server_password, SecretDraft::Clear);
        assert_eq!(state.focused_configuration_control, focused_before);
        assert_eq!(state.text_edit_session, edit_before);
        assert!(state.plex.authenticated);
        assert_eq!(state.plex.status, "error");
        assert!(state.plex.last_error.as_deref().is_some_and(|message| {
            message.contains("login was not saved") && message.contains("Connect to retry")
        }));
        assert_eq!(owner.plex_servers.len(), 1);
        assert!(state.has_unsaved_configuration_changes());
        assert!(handle.drain_actions().iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message,
            } if message.contains("login was not saved")
        )));
        assert!(root.is_dir());
        assert_eq!(
            std::fs::read_to_string(&sentinel_path).expect("sentinel should remain readable"),
            "unchanged"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
