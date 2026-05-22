use super::*;

impl GuiPersistedConfigRuntimeOwner {
    fn stream_helper_issue_notification_level(
        health: GuiStreamHelperHealth,
    ) -> GuiTransientNotificationLevel {
        match health {
            GuiStreamHelperHealth::Broken => GuiTransientNotificationLevel::Error,
            GuiStreamHelperHealth::Healthy
            | GuiStreamHelperHealth::MissingDownloader
            | GuiStreamHelperHealth::MissingJsRuntime
            | GuiStreamHelperHealth::Stale
            | GuiStreamHelperHealth::UnsupportedPlatform
            | GuiStreamHelperHealth::ExternalPlayerUnmanaged => {
                GuiTransientNotificationLevel::Warning
            }
        }
    }

    fn room_stream_target_kind(
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> GuiStreamTargetKind {
        let settings = state.configuration.to_stored_settings();
        let trusted_domains = settings.trusted_domains.unwrap_or_default();
        browser_stream_target_kind(
            target,
            Some((
                settings.only_switch_to_trusted_domains.unwrap_or(true),
                trusted_domains.as_slice(),
            )),
        )
    }

    fn queue_stream_support_feedback(
        &mut self,
        snapshot: crate::app::shell_state::GuiStreamHelperRuntimeSnapshot,
        user_initiated: bool,
        message_override: Option<String>,
    ) {
        let Some(message) = message_override.or_else(|| snapshot.message.clone()) else {
            return;
        };
        let mut actions = vec![GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
            snapshot.clone(),
        )];
        actions.push(GuiShellAction::PushTransientNotification {
            level: if user_initiated {
                Self::stream_helper_issue_notification_level(snapshot.health)
            } else {
                GuiTransientNotificationLevel::Warning
            },
            message: message.clone(),
        });
        actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        if user_initiated {
            actions.push(GuiShellAction::OpenModal(GuiShellModal::StreamSupport));
        }
        self.queue_stream_feedback_actions(actions);
    }

    pub(super) fn queue_stream_warning(&mut self, message: String) {
        self.queue_stream_feedback_actions(vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn queue_stream_error(&mut self, message: String) {
        self.queue_stream_feedback_actions(vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn prepare_stream_load_tracking(&mut self, target: &str, user_initiated: bool) {
        if browser_is_url(target) {
            self.pending_stream_load_context = Some(super::super::GuiPendingStreamLoadContext {
                requested_target: target.to_owned(),
                user_initiated,
            });
            if user_initiated {
                self.pending_stream_retry_target = Some(target.to_owned());
            }
        } else {
            self.pending_stream_load_context = None;
            if user_initiated {
                self.pending_stream_retry_target = None;
            }
        }
    }

    fn clear_pending_stream_load_context_for_target(&mut self, target: &str) {
        if self
            .pending_stream_load_context
            .as_ref()
            .is_some_and(|context| context.requested_target == target)
        {
            self.pending_stream_load_context = None;
        }
    }

    fn take_pending_stream_load_user_initiated_for_target(&mut self, target: &str) -> bool {
        if self
            .pending_stream_load_context
            .as_ref()
            .is_some_and(|context| context.requested_target == target)
        {
            return self
                .pending_stream_load_context
                .take()
                .is_some_and(|context| context.user_initiated);
        }
        false
    }

    pub(super) fn preflight_user_stream_target(&mut self, target: &str) -> bool {
        if browser_stream_target_kind(target, None) != GuiStreamTargetKind::ExtractorPageUrl {
            self.refresh_stream_helper_runtime_snapshot_for_target(None);
            return true;
        }
        let snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(Some(target));
        if snapshot.health == GuiStreamHelperHealth::Healthy {
            if let Err(error) = self.refresh_managed_player_if_stream_helper_refresh_required() {
                self.queue_stream_error(format!(
                    "Refreshing Sorotte-managed mpv for stream support failed: {error}"
                ));
                return false;
            }
            return true;
        }
        self.pending_stream_retry_target = Some(target.to_owned());
        self.queue_stream_support_feedback(snapshot, true, None);
        false
    }

    pub(super) fn preflight_room_stream_target(
        &mut self,
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> bool {
        match Self::room_stream_target_kind(state, target) {
            GuiStreamTargetKind::UntrustedUrl => {
                self.refresh_stream_helper_runtime_snapshot_for_target(None);
                self.queue_stream_warning(format!(
                    "Blocked automatic room URL open because the selected URL is not trusted locally: {target}."
                ));
                false
            }
            GuiStreamTargetKind::ExtractorPageUrl => {
                let snapshot = self.refresh_stream_helper_runtime_snapshot_for_target(Some(target));
                if snapshot.health == GuiStreamHelperHealth::Healthy {
                    if let Err(error) =
                        self.refresh_managed_player_if_stream_helper_refresh_required()
                    {
                        self.queue_stream_warning(format!(
                            "Automatic room URL open is unavailable locally: {error}"
                        ));
                        return false;
                    }
                    true
                } else {
                    let message = snapshot.message.as_ref().map(|message| {
                        format!("Automatic room URL open is unavailable locally: {message}")
                    });
                    self.queue_stream_support_feedback(snapshot, false, message);
                    false
                }
            }
            GuiStreamTargetKind::LocalPath | GuiStreamTargetKind::DirectMediaUrl => {
                self.refresh_stream_helper_runtime_snapshot_for_target(None);
                true
            }
        }
    }

    pub(super) fn handle_player_media_load_outcome(&mut self, outcome: PlayerMediaLoadOutcome) {
        if outcome.succeeded() {
            self.clear_pending_stream_load_context_for_target(&outcome.requested_target);
            if self.pending_stream_retry_target.as_deref()
                == Some(outcome.requested_target.as_str())
            {
                self.pending_stream_retry_target = None;
            }
            return;
        }

        self.player_local_file = None;
        self.player_local_file_placeholder = false;
        self.player_position_seconds = None;
        self.last_published_local_file = None;
        let user_initiated =
            self.take_pending_stream_load_user_initiated_for_target(&outcome.requested_target);
        let failure_message = outcome
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "The attached player reported a media load failure.".to_owned());

        if browser_stream_target_kind(&outcome.requested_target, None)
            == GuiStreamTargetKind::ExtractorPageUrl
        {
            let snapshot = self
                .refresh_stream_helper_runtime_snapshot_for_target(Some(&outcome.requested_target));
            if snapshot.health != GuiStreamHelperHealth::Healthy {
                self.pending_stream_retry_target = Some(outcome.requested_target.clone());
                let combined_message = snapshot.message.as_ref().map_or_else(
                    || failure_message.clone(),
                    |summary| {
                        if failure_message == *summary {
                            summary.clone()
                        } else {
                            format!("{summary} {failure_message}")
                        }
                    },
                );
                self.queue_stream_support_feedback(
                    snapshot,
                    user_initiated,
                    Some(combined_message),
                );
                return;
            }
        } else {
            self.refresh_stream_helper_runtime_snapshot_for_target(None);
        }

        let message = if browser_is_url(&outcome.requested_target) {
            format!("Loading media URL through the attached player failed: {failure_message}")
        } else {
            format!("Loading media through the attached player failed: {failure_message}")
        };
        if user_initiated {
            self.queue_stream_error(message);
        } else {
            self.queue_stream_warning(message);
        }
    }

    pub(super) fn open_media_files_through_attached_player_result_impl(
        &mut self,
        paths: &[String],
    ) -> Option<Result<String, String>> {
        if paths.is_empty() || self.player.is_none() {
            return None;
        }

        let selected_path = paths[0].clone();
        let (player_name, open_result) = {
            let player = self.player.as_mut()?;
            (player.name(), player.open_file(&selected_path))
        };
        Some(match open_result {
            Ok(()) => {
                self.player_local_file =
                    Some(Self::placeholder_local_file_for_path(&selected_path));
                self.player_local_file_placeholder = true;
                self.player_position_seconds = Some(0.0);
                self.refresh_player_state_impl();
                let preserve_ready_for_auto_advanced_playlist_item =
                    self.playlist_auto_advance_eof_latched
                        && self.session.as_ref().is_some_and(|session| {
                            session.has_pending_playlist_index_reset_intent()
                        });
                if !preserve_ready_for_auto_advanced_playlist_item
                    && let Some(session) = self.session.as_mut()
                {
                    let _ = session.mark_local_media_opened_not_ready();
                }
                if browser_is_url(&selected_path) && paths.len() == 1 {
                    Ok(format!(
                        "Started loading media URL through the attached {player_name} player: {selected_path}."
                    ))
                } else if browser_is_url(&selected_path) {
                    Ok(format!(
                        "Started loading the first selected media URL through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                        paths.len() - 1
                    ))
                } else if paths.len() == 1 {
                    Ok(format!(
                        "Opened media file through the attached {player_name} player: {selected_path}."
                    ))
                } else {
                    Ok(format!(
                        "Opened the first selected media file through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                        paths.len() - 1
                    ))
                }
            }
            Err(error) => {
                self.clear_pending_stream_load_context_for_target(&selected_path);
                Err(format!(
                    "Opening media through the attached {player_name} player failed: {error}"
                ))
            }
        })
    }

    pub(in crate::app::runtime_owner) fn open_media_files_through_attached_player_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        match self.open_media_files_through_attached_player_result_impl(&paths) {
            Some(Ok(message)) => Self::push_player_success_impl(handle, message),
            Some(Err(message)) => Self::push_player_error_impl(handle, message),
            None => {}
        }
    }
}
