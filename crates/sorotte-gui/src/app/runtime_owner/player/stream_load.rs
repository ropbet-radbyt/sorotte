use super::*;

use sorotte_plex::{PlexStreamTarget, redact_plex_token};

const MPV_FORCE_MEDIA_TITLE_OPTION: &str = "force-media-title";

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

    fn normalized_forced_media_title(title: &str) -> Option<String> {
        let normalized = title
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect::<String>();
        let trimmed = normalized.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn media_title_for_opened_path(path: &str) -> String {
        let placeholder = Self::placeholder_local_file_for_path(path);
        Self::normalized_forced_media_title(&redact_plex_token(&placeholder.name))
            .unwrap_or_else(|| "Media".to_owned())
    }

    fn media_title_for_plex_stream(stream_target: &PlexStreamTarget) -> String {
        [
            stream_target.playlist_uri.title.as_deref(),
            Some(stream_target.matched_item.title.as_str()),
            Some(stream_target.logical_file.name.as_str()),
        ]
        .into_iter()
        .flatten()
        .find_map(Self::normalized_forced_media_title)
        .unwrap_or_else(|| "Plex Stream".to_owned())
    }

    fn media_open_success_message(
        player_name: &str,
        selected_path: &str,
        selection_count: usize,
    ) -> String {
        let selected_path = redact_plex_token(selected_path);
        if browser_is_url(&selected_path) && selection_count == 1 {
            format!(
                "Started loading media URL through the attached {player_name} player: {selected_path}."
            )
        } else if browser_is_url(&selected_path) {
            format!(
                "Started loading the first selected media URL through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                selection_count - 1
            )
        } else if selection_count == 1 {
            format!("Opened media file through the attached {player_name} player: {selected_path}.")
        } else {
            format!(
                "Opened the first selected media file through the attached {player_name} player: {selected_path}. Ignored {} additional selections.",
                selection_count - 1
            )
        }
    }

    fn set_attached_player_forced_media_title(&mut self, title: &str) {
        let Some(title) = Self::normalized_forced_media_title(title) else {
            return;
        };
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let _ = player.set_option_string(MPV_FORCE_MEDIA_TITLE_OPTION, &title);
    }

    fn room_stream_target_kind(
        state: &SorotteGuiShellAppState,
        target: &str,
    ) -> GuiStreamTargetKind {
        let settings = state.configuration.to_stored_settings();
        let playback = ClientConfig::resolve(&settings).config.playback;
        browser_stream_target_kind(
            target,
            Some((
                playback.only_switch_to_trusted_domains,
                playback.trusted_domains.as_slice(),
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
        let message = redact_plex_token(&message);
        self.queue_stream_feedback_actions(vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn queue_stream_error(&mut self, message: String) {
        let message = redact_plex_token(&message);
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
                if !state
                    .plugin_enablement
                    .enabled_for(GuiPluginSelection::StreamSupport)
                {
                    self.refresh_stream_helper_runtime_snapshot_for_target(None);
                    self.queue_stream_warning(
                        "Automatic room URL open is unavailable locally: Stream Support is disabled."
                            .to_owned(),
                    );
                    return false;
                }
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
            GuiStreamTargetKind::LocalPath
            | GuiStreamTargetKind::DirectMediaUrl
            | GuiStreamTargetKind::PlexUri => {
                self.refresh_stream_helper_runtime_snapshot_for_target(None);
                true
            }
        }
    }

    fn pending_logical_media_override_matches_loaded_target(&self, target: &str) -> bool {
        self.pending_logical_media_override
            .as_ref()
            .is_some_and(|pending| pending.loaded_target_secret.as_str() == target)
    }

    fn take_pending_logical_media_failure_context(
        &mut self,
        target: &str,
    ) -> Option<(String, bool)> {
        if !self.pending_logical_media_override_matches_loaded_target(target) {
            return None;
        }
        self.pending_logical_media_override
            .take()
            .map(|pending| (pending.requested_target, pending.user_initiated))
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
        self.last_published_media_match_signature = None;
        let user_initiated =
            self.take_pending_stream_load_user_initiated_for_target(&outcome.requested_target);
        let logical_failure_context =
            self.take_pending_logical_media_failure_context(&outcome.requested_target);
        let requested_target = logical_failure_context
            .as_ref()
            .map(|(target, _)| target.as_str())
            .unwrap_or(outcome.requested_target.as_str());
        let user_initiated = user_initiated
            || logical_failure_context
                .as_ref()
                .is_some_and(|(_, user)| *user);
        let failure_message = outcome
            .failure
            .as_ref()
            .map(|failure| redact_plex_token(&failure.message))
            .unwrap_or_else(|| "The attached player reported a media load failure.".to_owned());

        if browser_stream_target_kind(requested_target, None)
            == GuiStreamTargetKind::ExtractorPageUrl
        {
            let snapshot =
                self.refresh_stream_helper_runtime_snapshot_for_target(Some(requested_target));
            if snapshot.health != GuiStreamHelperHealth::Healthy {
                self.pending_stream_retry_target = Some(requested_target.to_owned());
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

        let message = if browser_is_url(requested_target) {
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
        self.pending_logical_media_override = None;
        let (player_name, open_result) = {
            let player = self.player.as_mut()?;
            (player.name(), player.open_file_tracked(&selected_path))
        };
        Some(match open_result {
            Ok(()) => {
                let logical_file = Self::placeholder_local_file_for_path(&selected_path);
                let logical_id = logical_media_id_for_local_file_update(&logical_file);
                let kind = if browser_is_url(&selected_path) {
                    MediaTransportKind::NetworkVod
                } else {
                    MediaTransportKind::LocalFile
                };
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.prepare_attached_playback_media(
                        logical_id,
                        kind,
                        system_time_seconds(),
                    )
                {
                    eprintln!(
                        "warning: failed to prepare attached-player media coordination: {error}"
                    );
                }
                self.set_attached_player_forced_media_title(&Self::media_title_for_opened_path(
                    &selected_path,
                ));
                self.player_local_file =
                    Some(Self::placeholder_local_file_for_path(&selected_path));
                self.player_local_file_placeholder = browser_is_url(&selected_path);
                self.player_position_seconds = Some(0.0);
                self.player_paused_for_cache = None;
                self.player_cache_buffering_percent = None;
                self.pending_attached_room_unpause_observation = None;
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
                Ok(Self::media_open_success_message(
                    player_name,
                    &selected_path,
                    paths.len(),
                ))
            }
            Err(error) => {
                self.clear_pending_stream_load_context_for_target(&selected_path);
                Err(format!(
                    "Opening media through the attached {player_name} player failed: {}",
                    redact_plex_token(&error.to_string())
                ))
            }
        })
    }

    pub(in crate::app::runtime_owner) fn open_plex_stream_target_through_attached_player_result_impl(
        &mut self,
        requested_target: &str,
        stream_target: PlexStreamTarget,
        user_initiated: bool,
    ) -> Option<Result<String, String>> {
        self.player.as_ref()?;

        let loaded_target_secret = stream_target.playback_url.clone();
        let logical_file = stream_target.logical_file.clone();
        let logical_name = logical_file.name.clone();
        let media_title = Self::media_title_for_plex_stream(&stream_target);
        let (player_name, open_result) = {
            let player = self.player.as_mut()?;
            (
                player.name(),
                player.open_file_tracked(loaded_target_secret.as_str()),
            )
        };
        Some(match open_result {
            Ok(()) => {
                let logical_id = logical_media_id_for_local_file_update(&logical_file);
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.prepare_attached_playback_media(
                        logical_id,
                        MediaTransportKind::NetworkVod,
                        system_time_seconds(),
                    )
                {
                    eprintln!(
                        "warning: failed to prepare stable Plex media coordination identity: {error}"
                    );
                }
                self.set_attached_player_forced_media_title(&media_title);
                self.pending_stream_load_context = None;
                if user_initiated {
                    self.pending_stream_retry_target = None;
                }
                self.pending_logical_media_override =
                    Some(super::super::GuiPendingLogicalMediaOverride {
                        requested_target: requested_target.to_owned(),
                        loaded_target_secret,
                        logical_file: logical_file.clone(),
                        user_initiated,
                    });
                self.player_local_file = Some(logical_file);
                self.player_local_file_placeholder = false;
                self.player_position_seconds = Some(0.0);
                self.player_paused_for_cache = None;
                self.player_cache_buffering_percent = None;
                self.pending_attached_room_unpause_observation = None;
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
                Ok(format!(
                    "Started loading Plex media stream through the attached {player_name} player: {logical_name}."
                ))
            }
            Err(error) => {
                self.pending_logical_media_override = None;
                Err(format!(
                    "Opening Plex media stream through the attached {player_name} player failed: {}",
                    redact_plex_token(&error.to_string())
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

#[cfg(test)]
mod credential_feedback_tests {
    use super::*;

    #[test]
    fn media_open_success_feedback_redacts_plex_tokens_without_changing_local_paths() {
        let token = "stream-feedback-canary";
        let target =
            format!("https://media.example/video.m3u8?X-Plex-Token={token}&quality=original");
        let message = GuiPersistedConfigRuntimeOwner::media_open_success_message("mpv", &target, 1);
        assert!(message.contains("Started loading media URL"));
        assert!(!message.contains(token));

        let local = GuiPersistedConfigRuntimeOwner::media_open_success_message(
            "mpv",
            "C:/Media/movie.mkv",
            1,
        );
        assert!(local.contains("C:/Media/movie.mkv"));
    }
}
