use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn apply_media_match_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        progress: MediaMatchToolProgress,
    ) {
        self.report_media_match_remediation_progress(
            handle,
            projected_state,
            progress.label,
            progress.detail,
            progress.progress_fraction,
        );
    }

    fn finish_media_match_tool_success(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        success_message: String,
    ) {
        self.report_media_match_remediation_progress(
            handle,
            projected_state,
            "Rechecking Media Matching tools",
            Some("Verifying ffmpeg, ffprobe, and fpcalc.".to_owned()),
            0.92,
        );
        let snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        let actions = vec![
            GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: success_message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(success_message),
        ];
        Self::push_actions_and_project(handle, projected_state, actions);
        self.clear_media_match_remediation_progress(handle, projected_state);
    }

    fn finish_media_match_tool_error(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        failure_label: &str,
        error: String,
    ) {
        let message = format!("{failure_label}: {error}");
        let mut snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        snapshot.message = Some(message.clone());
        self.media_match_runtime_snapshot = snapshot.clone();
        self.clear_media_match_remediation_progress(handle, projected_state);
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(message),
            ],
        );
    }

    fn set_media_match_autoplay_gate(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        strong_same_media: bool,
    ) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) =
                session.set_strong_same_media_match_satisfies_filename_gate(strong_same_media)
        {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Could not update Media Matching autoplay gate: {error}"),
            );
            return false;
        }
        true
    }

    fn current_media_match_strong_for_autoplay(state: &SorotteGuiShellAppState) -> bool {
        state
            .media_match
            .settings
            .autoplay_allows_strong_same_media()
            && state
                .media_match
                .current_decision
                .as_deref()
                .is_some_and(|decision| decision.starts_with("strong:"))
    }

    pub(super) fn handle_install_media_match_tools_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Media Matching tool installation requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        self.report_media_match_remediation_progress(
            handle,
            projected_state,
            "Preparing Media Matching tools",
            Some(
                "Installing ffmpeg, ffprobe, and fpcalc into Sorotte's managed tools directory."
                    .to_owned(),
            ),
            0.02,
        );
        match install_or_update_managed_media_match_tools_with_progress(&root, |progress| {
            self.apply_media_match_progress(handle, projected_state, progress);
        }) {
            Ok(message) => self.finish_media_match_tool_success(handle, projected_state, message),
            Err(error) => self.finish_media_match_tool_error(
                handle,
                projected_state,
                "Media Matching tool install failed",
                error,
            ),
        }
        true
    }

    pub(super) fn handle_import_media_match_tool_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        tool: MediaMatchTool,
        source_path: String,
    ) -> bool {
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Media Matching tool import requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        self.report_media_match_remediation_progress(
            handle,
            projected_state,
            "Preparing Media Matching tool import",
            Some(source_path.clone()),
            0.02,
        );
        match import_managed_media_match_tool_with_progress(
            &root,
            tool,
            Path::new(&source_path),
            |progress| {
                self.apply_media_match_progress(handle, projected_state, progress);
            },
        ) {
            Ok(message) => self.finish_media_match_tool_success(handle, projected_state, message),
            Err(error) => self.finish_media_match_tool_error(
                handle,
                projected_state,
                "Media Matching tool import failed",
                error,
            ),
        }
        true
    }

    pub(super) fn handle_open_media_match_install_location_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Opening the Media Matching tools folder requires a writable GUI config root."
                    .to_owned(),
            );
            return false;
        };
        let install_location = managed_media_match_bin_dir(&root);
        if let Err(error) = std::fs::create_dir_all(&install_location) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not create the Media Matching tools folder '{}': {error}",
                    install_location.display()
                ),
            );
            return false;
        }
        self.open_stream_helper_install_location_runtime(handle, projected_state, install_location);
        true
    }

    pub(super) fn handle_recheck_media_match_tools_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        let mut actions = vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
            snapshot.clone(),
        )];
        if snapshot.health == GuiMediaMatchToolHealth::Healthy {
            let message = "Media Matching tools are ready.".to_owned();
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        } else if let Some(message) = snapshot.message.clone() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        true
    }

    pub(super) fn handle_rebuild_media_match_index_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Media Matching index rebuild requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        let search_roots = self.automatic_media_search_roots(projected_state);
        let current_player_path = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.clone());
        let settings = projected_state.media_match.settings.clone();
        self.report_media_match_remediation_progress(
            handle,
            projected_state,
            "Preparing Media Matching index",
            Some(format!("{} media-search roots", search_roots.len())),
            0.02,
        );
        let result = rebuild_persisted_media_match_index_with_progress(
            &root,
            &search_roots,
            current_player_path.as_deref(),
            &settings,
            |progress| {
                self.apply_media_match_progress(handle, projected_state, progress);
            },
        );
        self.clear_media_match_remediation_progress(handle, projected_state);
        match result {
            Ok(result) => {
                let mut snapshot = self
                    .refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
                snapshot.cache_status = Some(result.cache_status);
                snapshot.current_decision = result.current_decision;
                snapshot.last_evidence = result.last_evidence.or_else(|| {
                    Some(
                        "Fingerprint evidence is local-only; nothing is sent over Syncplay."
                            .to_owned(),
                    )
                });
                self.media_match_runtime_snapshot = snapshot.clone();
                let strong_same_media = projected_state
                    .media_match
                    .settings
                    .autoplay_allows_strong_same_media()
                    && snapshot
                        .current_decision
                        .as_deref()
                        .is_some_and(|decision| decision.starts_with("strong:"));
                if !self.set_media_match_autoplay_gate(handle, projected_state, strong_same_media) {
                    return true;
                }
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![
                        GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Info,
                            message: result.message.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(result.message),
                    ],
                );
            }
            Err(error) => {
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
        true
    }

    pub(super) fn handle_clear_media_match_cache_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if let Some(root) = self.legacy_gui_qsettings_root()
            && let Err(error) = clear_persisted_media_match_cache_at_root(&root)
        {
            Self::push_runtime_error_notification(handle, projected_state, error);
            return false;
        }
        let mut snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        snapshot.cache_status = Some("empty".to_owned());
        snapshot.current_decision = None;
        snapshot.last_evidence = None;
        self.media_match_runtime_snapshot = snapshot.clone();
        if !self.set_media_match_autoplay_gate(handle, projected_state, false) {
            return true;
        }
        let message = "Media Matching cache cleared.".to_owned();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(message),
            ],
        );
        true
    }

    pub(super) fn handle_set_media_match_fingerprinting_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        projected_state.media_match.settings.fingerprinting_enabled = enabled;
        self.persist_media_match_settings_request(handle, projected_state)
    }

    pub(super) fn handle_set_media_match_runtime_tolerance_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        projected_state
            .media_match
            .settings
            .runtime_tolerance_enabled = enabled;
        self.persist_media_match_settings_request(handle, projected_state)
    }

    pub(super) fn handle_set_media_match_autoplay_policy_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        policy: sorotte_media_match::MediaMatchAutoplayPolicy,
    ) -> bool {
        projected_state.media_match.settings.autoplay_policy = policy;
        self.persist_media_match_settings_request(handle, projected_state)
    }

    fn persist_media_match_settings_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        apply_media_match_settings_to_stored_settings(
            &mut projected_state.configuration.settings,
            &projected_state.media_match.settings,
        );
        if let Some(config_path) = self.config_path.as_ref()
            && let Err(error) = upsert_sorotte_ini_stored_client_settings_mvp_at_path(
                config_path,
                &projected_state.configuration.settings,
            )
        {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Could not persist Media Matching settings: {error}"),
            );
            return false;
        }
        let snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        let strong_same_media = Self::current_media_match_strong_for_autoplay(projected_state);
        if !self.set_media_match_autoplay_gate(handle, projected_state, strong_same_media) {
            return false;
        }
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![
                GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
                GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
                    GuiConfigurationRuntimeSnapshot {
                        draft_settings: projected_state.configuration.settings.clone(),
                        saved_settings: projected_state.configuration.settings.clone(),
                    },
                ),
            ],
        );
        true
    }
}
