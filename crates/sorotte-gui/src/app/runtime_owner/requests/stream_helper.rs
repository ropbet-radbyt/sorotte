use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn apply_stream_helper_remediation_progress(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        progress: StreamHelperRemediationProgress,
    ) {
        self.report_stream_helper_remediation_progress(
            handle,
            projected_state,
            progress.label,
            progress.detail,
            progress.progress_fraction,
        );
    }

    fn finish_stream_helper_remediation_success(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        success_message: String,
    ) {
        self.report_stream_helper_remediation_progress(
            handle,
            projected_state,
            "Rechecking stream helper support",
            Some("Verifying the updated helper against the current media URL.".to_owned()),
            0.88,
        );
        let snapshot = self.recheck_stream_helper_runtime_snapshot(projected_state);
        let mut actions = vec![
            GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(snapshot.clone()),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: success_message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(success_message),
        ];
        if snapshot.health != GuiStreamHelperHealth::Healthy {
            if let Some(issue_message) = snapshot.message.clone() {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: issue_message.clone(),
                });
                actions.push(GuiShellAction::AnnounceSystemChatEvent(issue_message));
            }
            Self::push_actions_and_project(handle, projected_state, actions);
            self.clear_stream_helper_remediation_progress(handle, projected_state);
            return;
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        self.mark_managed_player_stream_helper_refresh_required();

        let retry_target = self.pending_stream_retry_target.clone().or_else(|| {
            self.current_shared_playlist_target(projected_state)
                .filter(|target| {
                    browser_stream_target_kind(target, None)
                        == GuiStreamTargetKind::ExtractorPageUrl
                })
        });

        if let Some(target) = retry_target {
            self.report_stream_helper_remediation_progress(
                handle,
                projected_state,
                "Retrying media URL",
                Some(target.clone()),
                0.99,
            );
            self.open_main_window_user_media_runtime_impl(handle, projected_state, target);
        }
        self.clear_stream_helper_remediation_progress(handle, projected_state);
    }

    pub(super) fn handle_install_stream_helper_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Stream helper installation requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        self.report_stream_helper_remediation_progress(
            handle,
            projected_state,
            "Preparing stream helper remediation",
            Some("Starting helper installation for extractor-backed media URLs.".to_owned()),
            0.02,
        );
        match install_or_update_managed_stream_helper_with_progress(&root, |progress| {
            self.apply_stream_helper_remediation_progress(handle, projected_state, progress);
        }) {
            Ok(message) => {
                self.finish_stream_helper_remediation_success(handle, projected_state, message);
            }
            Err(error) => {
                self.clear_stream_helper_remediation_progress(handle, projected_state);
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
        true
    }

    pub(super) fn handle_open_stream_helper_install_location_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let install_location = projected_state
            .stream_helper
            .install_location
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                self.legacy_gui_qsettings_root()
                    .map(|root| managed_stream_helper_bin_dir(&root))
            });
        let Some(install_location) = install_location else {
            Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        "Opening the managed stream-helper install location requires a writable GUI config root."
                            .to_owned(),
                    );
            return false;
        };
        if let Err(error) = std::fs::create_dir_all(&install_location) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not create the managed stream-helper install location '{}': {error}",
                    install_location.display()
                ),
            );
            return false;
        }
        self.open_stream_helper_install_location_runtime(handle, projected_state, install_location);
        true
    }

    pub(super) fn handle_integrate_stream_helper_downloader_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        source_path: String,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Stream helper integration requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        self.report_stream_helper_remediation_progress(
            handle,
            projected_state,
            "Preparing stream helper remediation",
            Some("Starting yt-dlp import into Sorotte's managed helper.".to_owned()),
            0.02,
        );
        match import_managed_stream_helper_downloader_with_progress(
            &root,
            Path::new(&source_path),
            |progress| {
                self.apply_stream_helper_remediation_progress(handle, projected_state, progress);
            },
        ) {
            Ok(message) => {
                self.finish_stream_helper_remediation_success(handle, projected_state, message);
            }
            Err(error) => {
                self.clear_stream_helper_remediation_progress(handle, projected_state);
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
        true
    }

    pub(super) fn handle_integrate_stream_helper_js_runtime_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        source_path: String,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let Some(root) = self.legacy_gui_qsettings_root() else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Stream helper integration requires a writable GUI config root.".to_owned(),
            );
            return false;
        };
        self.report_stream_helper_remediation_progress(
            handle,
            projected_state,
            "Preparing stream helper remediation",
            Some("Starting Deno import into Sorotte's managed helper.".to_owned()),
            0.02,
        );
        match import_managed_stream_helper_js_runtime_with_progress(
            &root,
            Path::new(&source_path),
            |progress| {
                self.apply_stream_helper_remediation_progress(handle, projected_state, progress);
            },
        ) {
            Ok(message) => {
                self.finish_stream_helper_remediation_success(handle, projected_state, message);
            }
            Err(error) => {
                self.clear_stream_helper_remediation_progress(handle, projected_state);
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
        true
    }

    pub(super) fn handle_recheck_stream_helper_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let snapshot = self.recheck_stream_helper_runtime_snapshot(projected_state);
        let mut actions = vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            snapshot.clone(),
        )];
        if snapshot.health == GuiStreamHelperHealth::Healthy {
            let message = "Stream helper support is ready for the current media URL.".to_owned();
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

    pub(super) fn handle_retry_pending_stream_media_open_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::StreamSupport,
            );
            return true;
        }
        let Some(target) = self
            .pending_stream_retry_target
            .clone()
            .or_else(|| self.current_shared_playlist_target(projected_state))
        else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "No pending media URL is available to retry.".to_owned(),
            );
            return false;
        };
        Self::push_actions_and_project(handle, projected_state, vec![GuiShellAction::CloseModal]);
        self.open_main_window_user_media_runtime_impl(handle, projected_state, target);
        true
    }
}
