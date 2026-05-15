use std::{
    process::Command,
    time::{Duration, Instant, SystemTime},
};

use syncplay_plex::PlexServerConnectionKind;

use super::*;

const PLEX_AUTH_AUTO_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PLEX_WATCH_SYNC_PUMP_INTERVAL: Duration = Duration::from_secs(1);

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn handle_start_plex_auth_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
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
            .name("syncplay-gui-plex-auth-start".to_owned())
            .spawn(move || {
                let result = client.start_auth().map_err(|error| error.to_string());
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
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        self.poll_plex_auth(handle, projected_state, true);
        true
    }

    pub(super) fn pump_plex_auth_poll(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
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
        projected_state: &mut SyncplayGuiShellAppState,
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
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let settings = projected_state.configuration.to_stored_settings();
        if !self.startup_plex_server_refresh_attempted
            && settings
                .plex_user_token
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty())
        {
            self.startup_plex_server_refresh_attempted = true;
            if let Some(url) = settings.plex_selected_server_url.as_deref() {
                self.plex_server_reachability.insert(
                    plex_server_reachability_key(url),
                    GuiPlexServerReachability::Checking,
                );
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
            }
            let (tx, rx) = mpsc::channel();
            let startup_settings = settings.clone();
            match std::thread::Builder::new()
                .name("syncplay-gui-plex-startup".to_owned())
                .spawn(move || {
                    let result = PlexHttpClient::new("syncplay-rs-gui")
                        .map_err(|error| format!("Failed to create Plex HTTP client: {error}"))
                        .and_then(|client| {
                            refresh_plex_servers_and_reachability(&client, &startup_settings)
                        });
                    let _ = tx.send(result);
                }) {
                Ok(_thread) => {
                    self.startup_plex_server_refresh_rx = Some(rx);
                }
                Err(error) => self.apply_plex_error(
                    handle,
                    projected_state,
                    format!("Failed to start Plex server refresh at startup: {error}"),
                ),
            }
        }

        let Some(rx) = self.startup_plex_server_refresh_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(outcome)) => {
                self.apply_plex_server_refresh_outcome(outcome);
                let mut settings = projected_state.configuration.to_stored_settings();
                if reconcile_plex_server_selection(&mut settings, &self.plex_servers, true) {
                    self.persist_plex_settings_and_project(handle, projected_state, settings);
                }
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
            }
            Ok(Err(error)) => {
                if let Some(url) = projected_state
                    .configuration
                    .to_stored_settings()
                    .plex_selected_server_url
                    .as_deref()
                {
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
            Err(mpsc::TryRecvError::Empty) => {
                self.startup_plex_server_refresh_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {}
        }
    }

    pub(super) fn pump_plex_server_refresh(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(rx) = self.plex_server_refresh_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(outcome)) => {
                let context = self
                    .plex_server_refresh_context
                    .take()
                    .unwrap_or(GuiPlexServerRefreshContext::Manual);
                self.apply_plex_server_refresh_outcome(outcome);
                let mut settings = projected_state.configuration.to_stored_settings();
                let selection_changed =
                    reconcile_plex_server_selection(&mut settings, &self.plex_servers, true);
                let has_selected_server = settings.plex_selected_server_url.is_some();
                if selection_changed {
                    self.persist_plex_settings_and_project(handle, projected_state, settings);
                }
                self.sync_plex_runtime_snapshot(handle, projected_state, None);
                if self.plex_servers.is_empty() && !has_selected_server {
                    let message = match context {
                        GuiPlexServerRefreshContext::Manual => {
                            "No reachable Plex Media Servers were returned for this account."
                        }
                        GuiPlexServerRefreshContext::Login => {
                            "Plex login succeeded, but no reachable Plex Media Servers were returned for this account."
                        }
                    };
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
            Ok(Err(error)) => {
                let context = self
                    .plex_server_refresh_context
                    .take()
                    .unwrap_or(GuiPlexServerRefreshContext::Manual);
                match context {
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
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_server_refresh_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.plex_server_refresh_context = None;
                self.apply_plex_error(
                    handle,
                    projected_state,
                    "Plex server refresh worker stopped before returning a result.".to_owned(),
                );
            }
        }
    }

    fn poll_plex_auth(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
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
            .name("syncplay-gui-plex-auth-poll".to_owned())
            .spawn(move || {
                let result = client.poll_auth(pin_id).map_err(|error| error.to_string());
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
        projected_state: &mut SyncplayGuiShellAppState,
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
        projected_state: &mut SyncplayGuiShellAppState,
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
        self.plex_auth_session = None;
        self.plex_auth_poll_due_at = None;
        let mut settings = projected_state.configuration.to_stored_settings();
        settings.plex_user_token = Some(token);
        settings.plex_sync_enabled.get_or_insert(false);
        self.startup_plex_server_refresh_rx = None;
        self.plex_server_refresh_rx = None;
        self.plex_server_refresh_context = None;
        let refresh_start_error = self
            .start_plex_server_refresh_worker(&settings, GuiPlexServerRefreshContext::Login)
            .err();
        self.persist_plex_settings_and_project(handle, projected_state, settings);
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
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        if self.plex_server_refresh_rx.is_some() {
            return true;
        }
        let settings = projected_state.configuration.to_stored_settings();
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
        projected_state: &mut SyncplayGuiShellAppState,
        machine_identifier: String,
        uri: String,
    ) -> bool {
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
        let mut settings = projected_state.configuration.to_stored_settings();
        apply_plex_server_to_settings(&mut settings, &server);
        self.persist_plex_settings_and_project(handle, projected_state, settings);
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_toggle_plex_sync_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        enabled: bool,
    ) -> bool {
        let mut settings = projected_state.configuration.to_stored_settings();
        settings.plex_sync_enabled = Some(enabled);
        self.persist_plex_settings_and_project(handle, projected_state, settings);
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn handle_disconnect_plex_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) -> bool {
        self.plex_auth_session = None;
        self.plex_auth_start_rx = None;
        self.plex_auth_poll_rx = None;
        self.plex_auth_poll_due_at = None;
        self.plex_servers.clear();
        self.plex_server_reachability.clear();
        self.startup_plex_server_refresh_rx = None;
        self.plex_server_refresh_rx = None;
        self.plex_server_refresh_context = None;
        self.plex_sync_engine = None;
        self.plex_sync_rx = None;
        self.plex_sync_next_tick_due_at = None;
        let mut settings = projected_state.configuration.to_stored_settings();
        settings.plex_sync_enabled = Some(false);
        settings.plex_user_token = None;
        settings.plex_selected_server_id = None;
        settings.plex_selected_server_url = None;
        settings.plex_selected_server_token = None;
        self.persist_plex_settings_and_project_clearing_plex_identity(
            handle,
            projected_state,
            settings,
        );
        self.sync_plex_runtime_snapshot(handle, projected_state, None);
        true
    }

    pub(super) fn sync_plex_watch_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        self.drain_plex_sync_worker(handle, projected_state);
        if self.plex_sync_rx.is_some() {
            return;
        }
        let settings = projected_state.configuration.to_stored_settings();
        let config = plex_config_from_settings(&settings);
        if !config.enabled || !config.has_selected_server() {
            self.plex_sync_next_tick_due_at = None;
            if let Some(engine) = self.plex_sync_engine.as_mut() {
                engine.set_config(config);
            }
            self.sync_plex_runtime_snapshot(handle, projected_state, None);
            return;
        }
        let now = Instant::now();
        if let Some(due_at) = self.plex_sync_next_tick_due_at
            && now < due_at
        {
            return;
        }

        let event = self.player_local_file.clone().map(|file| {
            let mut event = PlexWatchEvent::new(file).with_changed_at(SystemTime::now());
            if let Some(position) = self.player_position_seconds {
                event = event.with_position_seconds(position);
            }
            if let Some(paused) = self.player_paused {
                event = event.with_paused(paused);
            }
            event
        });
        let cache_path = self.plex_cache_path();
        let engine = match self.take_plex_sync_engine(config) {
            Ok(engine) => engine,
            Err(message) => {
                self.apply_plex_error(handle, projected_state, message);
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("syncplay-gui-plex-watch-sync".to_owned())
            .spawn(move || {
                let mut engine = engine;
                let before = engine.cache().clone();
                let status = engine.tick(event, SystemTime::now());
                let cache_save_error = if engine.cache() != &before {
                    cache_path.and_then(|path| {
                        engine
                            .cache()
                            .save_to_path(&path)
                            .err()
                            .map(|error| format!("Failed to save Plex match cache: {error}"))
                    })
                } else {
                    None
                };
                let _ = tx.send(GuiPlexSyncWorkerResult {
                    engine,
                    status,
                    cache_save_error,
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
    }

    fn ensure_plex_client(&mut self) -> Result<&PlexHttpClient, String> {
        if self.plex_client.is_none() {
            self.plex_client = Some(
                PlexHttpClient::new("syncplay-rs-gui")
                    .map_err(|error| format!("Failed to create Plex HTTP client: {error}"))?,
            );
        }
        self.plex_client
            .as_ref()
            .ok_or_else(|| "Failed to create Plex HTTP client.".to_owned())
    }

    fn take_plex_sync_engine(
        &mut self,
        config: PlexClientConfig,
    ) -> Result<PlexSyncEngine<PlexHttpClient>, String> {
        if self.plex_sync_engine.is_none() {
            let client = self.ensure_plex_client()?.clone();
            let cache = self
                .plex_cache_path()
                .map(|path| PlexMatchCache::load_from_path(&path))
                .transpose()
                .map_err(|error| format!("Failed to load Plex match cache: {error}"))?
                .unwrap_or_default();
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
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let Some(rx) = self.plex_sync_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.plex_sync_engine = Some(result.engine);
                if let Some(error) = result.cache_save_error {
                    self.apply_plex_error(handle, projected_state, error);
                } else {
                    self.sync_plex_runtime_snapshot(handle, projected_state, Some(result.status));
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.plex_sync_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => self.apply_plex_error(
                handle,
                projected_state,
                "Plex sync worker stopped before returning a result.".to_owned(),
            ),
        }
    }

    fn start_plex_server_refresh_worker(
        &mut self,
        settings: &StoredClientSettingsMvp,
        context: GuiPlexServerRefreshContext,
    ) -> Result<(), String> {
        let client = self.ensure_plex_client().map(Clone::clone)?;
        let settings = settings.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("syncplay-gui-plex-server-refresh".to_owned())
            .spawn(move || {
                let result = refresh_plex_servers_and_reachability(&client, &settings);
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.plex_server_refresh_rx = Some(rx);
        self.plex_server_refresh_context = Some(context);
        Ok(())
    }

    fn apply_plex_server_refresh_outcome(&mut self, outcome: GuiPlexServerRefreshOutcome) {
        self.plex_servers = outcome.servers;
        self.plex_server_reachability = outcome.reachability;
    }

    fn persist_plex_settings_and_project(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        settings: StoredClientSettingsMvp,
    ) {
        self.persist_plex_settings_and_project_with_identity_clear(
            handle,
            projected_state,
            settings,
            false,
        );
    }

    fn persist_plex_settings_and_project_clearing_plex_identity(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        settings: StoredClientSettingsMvp,
    ) {
        self.persist_plex_settings_and_project_with_identity_clear(
            handle,
            projected_state,
            settings,
            true,
        );
    }

    fn persist_plex_settings_and_project_with_identity_clear(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        settings: StoredClientSettingsMvp,
        clear_plex_identity: bool,
    ) {
        if let Some(path) = self.config_path.as_ref() {
            let persist_result = if clear_plex_identity {
                syncplay_client_app::app_boundary::persistence::upsert_syncplay_ini_stored_client_settings_mvp_clearing_plex_identity_at_path(
                    path, &settings,
                )
            } else {
                syncplay_client_app::app_boundary::persistence::upsert_syncplay_ini_stored_client_settings_mvp_at_path(
                    path, &settings,
                )
            };
            if let Err(error) = persist_result {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Warning,
                        message: format!("Plex settings changed but could not be saved: {error}"),
                    }],
                );
            }
        }
        let snapshot = GuiConfigurationRuntimeSnapshot {
            draft_settings: settings.clone(),
            saved_settings: settings,
        };
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
                snapshot,
            )],
        );
    }

    fn sync_plex_runtime_snapshot(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        status: Option<PlexSyncStatus>,
    ) {
        let settings = projected_state.configuration.to_stored_settings();
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
        let selected_server_id = settings.plex_selected_server_id.clone();
        let selected_server_url = settings.plex_selected_server_url.clone();
        let authenticated = settings
            .plex_user_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
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
            enabled: settings.plex_sync_enabled.unwrap_or(false),
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
        projected_state: &mut SyncplayGuiShellAppState,
        message: String,
    ) {
        self.plex_runtime_snapshot.last_error = Some(message.clone());
        let mut snapshot = self.plex_snapshot_from_settings_and_status(
            &projected_state.configuration.to_stored_settings(),
            None,
        );
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

    fn plex_cache_path(&self) -> Option<PathBuf> {
        self.config_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|parent| parent.join("plex-watch-cache.json"))
    }

    fn schedule_next_plex_auth_poll(&mut self) {
        self.plex_auth_poll_due_at = Some(Instant::now() + PLEX_AUTH_AUTO_POLL_INTERVAL);
    }
}

fn plex_config_from_settings(settings: &StoredClientSettingsMvp) -> PlexClientConfig {
    PlexClientConfig {
        enabled: settings.plex_sync_enabled.unwrap_or(false),
        user_token: settings.plex_user_token.clone(),
        selected_server_id: settings.plex_selected_server_id.clone(),
        selected_server_url: settings.plex_selected_server_url.clone(),
        selected_server_token: settings.plex_selected_server_token.clone(),
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
    if selected_id.is_some_and(|id| {
        selected_url.is_some_and(|url| {
            servers
                .iter()
                .any(|server| server.machine_identifier == id && server.uri == url)
        })
    }) {
        return false;
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

fn refresh_plex_servers_and_reachability(
    client: &PlexHttpClient,
    settings: &StoredClientSettingsMvp,
) -> Result<GuiPlexServerRefreshOutcome, String> {
    let token = settings
        .plex_user_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Plex login is required before servers can be refreshed.".to_owned())?
        .to_owned();
    let servers = client
        .discover_servers(&token)
        .map_err(|error| error.to_string())?;
    let mut reachability = HashMap::new();
    for server in &servers {
        reachability.insert(
            plex_server_reachability_key(&server.uri),
            verify_plex_server_reachability(client, server),
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
                .filter(|token| !token.trim().is_empty())
                .unwrap_or(token),
            owned: true,
            has_local_connection: plex_server_connection_kind_from_uri(uri)
                == PlexServerConnectionKind::Local,
            connection_kind: plex_server_connection_kind_from_uri(uri),
        };
        reachability.insert(
            plex_server_reachability_key(&selected_server.uri),
            verify_plex_server_reachability(client, &selected_server),
        );
    }

    Ok(GuiPlexServerRefreshOutcome {
        servers,
        reachability,
    })
}

fn verify_plex_server_reachability(
    client: &PlexHttpClient,
    server: &PlexServerConnection,
) -> GuiPlexServerReachability {
    match client.verify_server_connection(server) {
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

    #[test]
    fn open_system_url_command_preserves_plex_auth_url_as_one_argument() {
        let url = "https://app.plex.tv/auth#?clientID=syncplay&code=ABCD&context%5Bdevice%5D%5Bproduct%5D=Syncplay%20Rust";
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
            plex_selected_server_token: Some("old-token".to_owned()),
            ..StoredClientSettingsMvp::default()
        };
        let servers = vec![PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: "https://125-209-152-187.raptor-machine.plex.direct:32400".to_owned(),
            access_token: "new-token".to_owned(),
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
            settings.plex_selected_server_token.as_deref(),
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
            plex_user_token: Some("user-token".to_owned()),
            plex_selected_server_id: Some("raptor-machine".to_owned()),
            plex_selected_server_url: Some("https://raptor.example:32400".to_owned()),
            plex_selected_server_token: Some("server-token".to_owned()),
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
            access_token: "other-token".to_owned(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        });
        owner.plex_server_reachability.insert(
            plex_server_reachability_key("https://saved.example:32400"),
            GuiPlexServerReachability::Reachable,
        );
        let settings = StoredClientSettingsMvp {
            plex_user_token: Some("user-token".to_owned()),
            plex_selected_server_id: Some("saved-machine".to_owned()),
            plex_selected_server_url: Some("https://saved.example:32400".to_owned()),
            plex_selected_server_token: Some("saved-token".to_owned()),
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
    fn start_plex_server_refresh_worker_supersedes_pending_refresh() {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let (_tx, rx) = mpsc::channel();
        owner.plex_server_refresh_rx = Some(rx);
        owner.plex_server_refresh_context = Some(GuiPlexServerRefreshContext::Manual);

        owner
            .start_plex_server_refresh_worker(
                &StoredClientSettingsMvp::default(),
                GuiPlexServerRefreshContext::Login,
            )
            .expect("refresh worker should start");

        assert!(owner.plex_server_refresh_rx.is_some());
        assert_eq!(
            owner.plex_server_refresh_context,
            Some(GuiPlexServerRefreshContext::Login)
        );
    }
}
