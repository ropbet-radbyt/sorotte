use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use sorotte_media_match::{
    MediaExtractionSettings, MediaMatchDecision, MediaMatchTier, MediaMatchWireSignature,
    decide_media_match_against_wire_signature,
};

use crate::app::media_match_support::{
    discard_media_match_index_rebuild_backup, media_match_record_for_path,
    media_match_sqlite_index_exists, media_match_tier_label,
    prepare_media_match_index_rebuild_backup, restore_media_match_index_rebuild_backup,
};

use super::super::{
    GuiMediaMatchBackgroundCancelDisposition, GuiMediaMatchBackgroundWorkerEvent,
    GuiMediaMatchIndexRebuildBackup, GuiMediaMatchRemoteLookupResult, GuiMediaMatchToolWorkerEvent,
};
use super::*;

const MEDIA_MATCH_BACKGROUND_EVENTS_PER_PUMP: usize = 64;

#[derive(Debug, Clone)]
struct GuiMediaMatchRemoteTarget {
    target_file_name: String,
    media_match_signature: MediaMatchWireSignature,
}

#[derive(Debug)]
struct GuiMediaMatchRemoteLookupRequest {
    root: PathBuf,
    search_roots: Vec<PathBuf>,
    candidate_paths: Option<Vec<PathBuf>>,
    remote: GuiMediaMatchRemoteTarget,
    trigger_key: String,
}

fn media_match_sampled_fast_extraction_settings() -> MediaExtractionSettings {
    MediaExtractionSettings::sampled_fast_audio_index_v3()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuiMediaMatchExactPlaylistPlan {
    None,
    ExactNoFingerprint { path: String },
    ExactNeedsSignature { path: String },
}

impl GuiPersistedConfigRuntimeOwner {
    fn media_match_resolution_enabled(projected_state: &SorotteGuiShellAppState) -> bool {
        projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
            && projected_state.media_match.settings.fingerprinting_enabled
    }

    fn usable_media_match_peer_file_name(file_name: Option<String>) -> Option<String> {
        file_name
            .map(|target| target.trim().to_owned())
            .filter(|target| !target.is_empty() && target != "**Hidden filename**")
    }

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
            Some("Verifying ffmpeg and ffprobe for V3.".to_owned()),
            0.92,
        );
        let snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
        self.media_match_wire_sync_token = None;
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

    fn set_media_match_peer_tiers(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        tiers: BTreeMap<String, MediaMatchTier>,
    ) -> bool {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.set_media_match_peer_tiers(tiers)
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

    fn summarize_media_match_wire_decision(
        username: &str,
        decision: &MediaMatchDecision,
    ) -> String {
        let tier = media_match_tier_label(decision.tier);
        format!("{username}: {tier}")
    }

    fn sync_media_match_wire_decisions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let sync_token = self.media_match_wire_sync_token_for_state(projected_state);
        self.media_match_wire_sync_token = Some(sync_token);
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            if !self.set_media_match_peer_tiers(handle, projected_state, BTreeMap::new()) {
                return false;
            }
            self.update_media_match_remote_status(
                handle,
                projected_state,
                "disabled: plugin off".to_owned(),
            );
            return true;
        }
        let mut tiers = BTreeMap::new();
        let status = if !projected_state.media_match.settings.fingerprinting_enabled {
            "disabled: fingerprinting off".to_owned()
        } else if !projected_state.media_match.settings.wire_sharing_enabled {
            "disabled: sharing off".to_owned()
        } else {
            let Some(root) = self.media_match_root_for_request(projected_state) else {
                let status = "unavailable: no storage root".to_owned();
                let gate_tiers = BTreeMap::new();
                if !self.set_media_match_peer_tiers(handle, projected_state, gate_tiers) {
                    return false;
                }
                self.update_media_match_remote_status(handle, projected_state, status);
                return true;
            };
            let current_path =
                if let Some(path) = self.media_match_wire_local_path_for_state(projected_state) {
                    path
                } else if let Some(path) = self
                    .media_match_room_target_for_state(projected_state)
                    .and_then(|target| {
                        self.media_match_cached_room_candidate_for_target(projected_state, &target)
                    })
                {
                    path
                } else {
                    let status = "unavailable: no current file".to_owned();
                    let gate_tiers = BTreeMap::new();
                    if !self.set_media_match_peer_tiers(handle, projected_state, gate_tiers) {
                        return false;
                    }
                    self.update_media_match_remote_status(handle, projected_state, status);
                    return true;
                };
            let Some(local_record) = media_match_record_for_path(
                &root,
                &current_path,
                &media_match_sampled_fast_extraction_settings(),
            ) else {
                let status = if self.media_match_runtime_snapshot.health
                    == GuiMediaMatchToolHealth::Healthy
                {
                    "pending local fingerprint".to_owned()
                } else {
                    "unavailable: tools unhealthy".to_owned()
                };
                let gate_tiers = BTreeMap::new();
                if !self.set_media_match_peer_tiers(handle, projected_state, gate_tiers) {
                    return false;
                }
                self.update_media_match_remote_status(handle, projected_state, status);
                return true;
            };
            let remote_peer_states = self
                .session
                .as_ref()
                .map(|session| session.current_room_media_match_peer_file_states())
                .unwrap_or_default();
            if remote_peer_states.is_empty() {
                "unavailable: no room peers".to_owned()
            } else {
                let mut summaries = Vec::new();
                for peer_state in remote_peer_states {
                    let username = peer_state.username;
                    let Some(signature) = peer_state.media_match_signature else {
                        summaries.push(format!("{username}: unavailable"));
                        continue;
                    };
                    let decision = decide_media_match_against_wire_signature(
                        &local_record,
                        &signature,
                        &projected_state.media_match.settings,
                    );
                    tiers.insert(username.clone(), decision.tier);
                    summaries.push(Self::summarize_media_match_wire_decision(
                        &username, &decision,
                    ));
                }
                summaries.join(", ")
            }
        };

        let gate_tiers = if projected_state
            .media_match
            .settings
            .autoplay_allows_strong_same_media()
        {
            tiers
        } else {
            BTreeMap::new()
        };
        if !self.set_media_match_peer_tiers(handle, projected_state, gate_tiers) {
            return false;
        }
        self.update_media_match_remote_status(handle, projected_state, status);
        true
    }

    pub(in crate::app::runtime_owner) fn maybe_sync_media_match_wire_decisions(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let token = self.media_match_wire_sync_token_for_state(projected_state);
        if self.media_match_wire_sync_token.as_deref() == Some(token.as_str()) {
            return true;
        }
        self.sync_media_match_wire_decisions(handle, projected_state)
    }

    fn media_match_wire_sync_token_for_state(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> String {
        let current_path = self
            .media_match_wire_local_path_for_state(projected_state)
            .unwrap_or_default();
        let remote_peer_states = self
            .session
            .as_ref()
            .map(|session| session.current_room_media_match_peer_file_states())
            .unwrap_or_default();
        let remote_signature_token = format!("{remote_peer_states:?}");
        format!(
            "{}|{}|{}|{}|{:?}|{:?}|{}",
            current_path,
            projected_state
                .plugin_enablement
                .enabled_for(GuiPluginSelection::MediaMatching),
            projected_state.media_match.settings.fingerprinting_enabled,
            projected_state.media_match.settings.wire_sharing_enabled,
            projected_state.media_match.settings.autoplay_policy,
            self.media_match_runtime_snapshot.health,
            remote_signature_token
        )
    }

    fn update_media_match_remote_status(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        status: String,
    ) {
        if self.media_match_runtime_snapshot.remote_status.as_deref() == Some(status.as_str())
            && self.media_match_runtime_snapshot.settings == projected_state.media_match.settings
        {
            return;
        }
        let mut snapshot = self.media_match_runtime_snapshot.clone();
        snapshot.settings = projected_state.media_match.settings.clone();
        snapshot.remote_status = Some(status);
        self.media_match_runtime_snapshot = snapshot.clone();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)],
        );
    }

    fn media_match_tool_worker_busy_notification(
        &self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if self.media_match_tool_worker_rx.is_none() {
            return false;
        }
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Media Matching tool installation or import is already running."
                    .to_owned(),
            }],
        );
        true
    }

    fn media_match_config_path_for_request(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<PathBuf> {
        if let Some(config_path) = self.config_path.clone() {
            return Some(config_path);
        }
        let config_path = projected_state
            .config_storage
            .config_path
            .as_deref()
            .map(PathBuf::from)
            .or_else(resolve_sorotte_gui_config_path_legacy_compatible)?;
        self.config_path = Some(config_path.clone());
        Some(config_path)
    }

    pub(in crate::app) fn media_match_root_for_request(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<PathBuf> {
        self.media_match_config_path_for_request(projected_state)
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    pub(in crate::app::runtime_owner) fn pump_media_match_tool_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(rx) = self.media_match_tool_worker_rx.take() else {
            return;
        };
        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(GuiMediaMatchToolWorkerEvent::Progress(progress)) => {
                    self.apply_media_match_progress(handle, projected_state, progress);
                }
                Ok(GuiMediaMatchToolWorkerEvent::Finished {
                    result,
                    failure_label,
                }) => {
                    keep_rx = false;
                    match result {
                        Ok(message) => {
                            self.finish_media_match_tool_success(handle, projected_state, message)
                        }
                        Err(error) => self.finish_media_match_tool_error(
                            handle,
                            projected_state,
                            failure_label,
                            error,
                        ),
                    }
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    keep_rx = false;
                    self.finish_media_match_tool_error(
                        handle,
                        projected_state,
                        "Media Matching tool operation failed",
                        "worker stopped before reporting a result".to_owned(),
                    );
                    break;
                }
            }
        }
        if keep_rx {
            self.media_match_tool_worker_rx = Some(rx);
        }
    }

    fn media_match_background_progress_status(progress: &MediaMatchToolProgress) -> String {
        match progress
            .detail
            .as_deref()
            .filter(|detail| !detail.is_empty())
        {
            Some(detail) => format!("{}: {detail}", progress.label),
            None => progress.label.clone(),
        }
    }

    fn publish_media_match_background_status(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        status: impl Into<String>,
    ) {
        let mut snapshot = self.media_match_runtime_snapshot.clone();
        snapshot.background_status = Some(status.into());
        self.media_match_runtime_snapshot = snapshot.clone();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)],
        );
    }

    pub(in crate::app::runtime_owner) fn request_media_match_background_worker_cancel(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        disposition: GuiMediaMatchBackgroundCancelDisposition,
        status: impl Into<String>,
    ) -> bool {
        if let Some(cancel_flag) = self.media_match_background_worker_cancel.as_ref() {
            cancel_flag.store(true, Ordering::Relaxed);
            self.media_match_background_cancel_disposition = Some(disposition);
            self.publish_media_match_background_status(handle, projected_state, status);
            return true;
        }
        false
    }

    fn finish_media_match_background_index_backup(
        &mut self,
        preserve_new_index: bool,
    ) -> Result<(), String> {
        let Some(backup) = self.media_match_background_index_backup.take() else {
            return Ok(());
        };
        if preserve_new_index {
            discard_media_match_index_rebuild_backup(&backup.root)
        } else {
            restore_media_match_index_rebuild_backup(&backup.root, backup.backup_existed)
        }
    }

    fn publish_media_match_background_cancel_status(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        disposition: GuiMediaMatchBackgroundCancelDisposition,
    ) {
        let restore_result = self.finish_media_match_background_index_backup(
            disposition == GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
        );
        let status = match (disposition, restore_result) {
            (GuiMediaMatchBackgroundCancelDisposition::RestorePrevious, Ok(())) => {
                "canceled: previous index restored".to_owned()
            }
            (GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint, Ok(())) => {
                "canceled: checkpoint kept".to_owned()
            }
            (_, Err(error)) => {
                format!("canceled: restore failed: {error}")
            }
        };
        let mut snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        snapshot.background_status = Some(status);
        self.media_match_runtime_snapshot = snapshot.clone();
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)],
        );
    }

    fn media_match_background_trigger_key(
        &self,
        projected_state: &SorotteGuiShellAppState,
        search_roots: &[PathBuf],
        current_player_path: Option<&str>,
    ) -> String {
        let current_player_path = current_player_path.unwrap_or_default();
        let room_target = self
            .media_match_room_target_for_state(projected_state)
            .unwrap_or_default();
        let remote_targets = format!(
            "{:?}",
            self.media_match_remote_targets_for_state(projected_state)
        );
        let roots = search_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let settings = &projected_state.media_match.settings;
        format!(
            "current={current_player_path}\ntarget={room_target}\nremote={remote_targets}\nroots={roots}\nfingerprinting={}\nruntime={}\nautoplay={:?}\nwarmup={}",
            settings.fingerprinting_enabled,
            settings.runtime_tolerance_enabled,
            settings.autoplay_policy,
            settings.background_warmup_enabled,
        )
    }

    fn media_match_exact_playlist_signature_trigger_key(path: &str) -> String {
        format!("exact-playlist-signature={path}")
    }

    fn local_shared_playlist_media_match_signature_path_matches(&self, path: &str) -> bool {
        self.local_shared_playlist_media_match_signature_path
            .as_deref()
            .is_some_and(|source_path| {
                Self::normalized_current_player_match_key(source_path)
                    == Self::normalized_current_player_match_key(path)
            })
    }

    pub(in crate::app::runtime_owner) fn remember_local_shared_playlist_media_match_signature_path(
        &mut self,
        path: &str,
    ) {
        self.local_shared_playlist_media_match_signature_path = Some(path.to_owned());
    }

    pub(in crate::app) fn clear_local_shared_playlist_media_match_signature_path_if_current(
        &mut self,
        local_file: Option<&sorotte_player_api::LocalFileUpdate>,
    ) {
        let Some(path) = local_file.and_then(|file| file.path.as_deref()) else {
            return;
        };
        if self.local_shared_playlist_media_match_signature_path_matches(path) {
            self.local_shared_playlist_media_match_signature_path = None;
        }
    }

    pub(in crate::app) fn media_match_wire_signature_allowed_for_local_file(
        &self,
        projected_state: &SorotteGuiShellAppState,
        local_file: Option<&sorotte_player_api::LocalFileUpdate>,
    ) -> bool {
        let Some(path) = local_file.and_then(|file| file.path.as_deref()) else {
            return false;
        };
        let Some(target) = self.current_shared_playlist_target(projected_state) else {
            return true;
        };
        if !self.current_player_matches_media_target(&target) {
            return true;
        }
        self.local_shared_playlist_media_match_signature_path_matches(path)
    }

    fn media_match_exact_playlist_plan_for_state(
        &self,
        projected_state: &SorotteGuiShellAppState,
        root: &Path,
    ) -> GuiMediaMatchExactPlaylistPlan {
        let Some(target) = self.current_shared_playlist_target(projected_state) else {
            return GuiMediaMatchExactPlaylistPlan::None;
        };
        if !self.current_player_matches_media_target(&target) {
            return GuiMediaMatchExactPlaylistPlan::None;
        }
        let Some(path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .filter(|path| Path::new(path).is_file())
            .map(str::to_owned)
        else {
            return GuiMediaMatchExactPlaylistPlan::None;
        };

        if projected_state.media_match.settings.wire_sharing_enabled
            && self.local_shared_playlist_media_match_signature_path_matches(&path)
            && media_match_record_for_path(
                root,
                &path,
                &media_match_sampled_fast_extraction_settings(),
            )
            .is_none()
        {
            return GuiMediaMatchExactPlaylistPlan::ExactNeedsSignature { path };
        }

        GuiMediaMatchExactPlaylistPlan::ExactNoFingerprint { path }
    }

    fn queue_exact_playlist_signature_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        root: PathBuf,
        path: String,
        force_restart: bool,
        notify_on_finish: bool,
    ) -> bool {
        let trigger_key = Self::media_match_exact_playlist_signature_trigger_key(&path);
        if self.media_match_background_worker_rx.is_some() {
            if !force_restart
                && self.media_match_background_trigger_key.as_deref() == Some(trigger_key.as_str())
            {
                return true;
            }
            let disposition = if force_restart {
                GuiMediaMatchBackgroundCancelDisposition::RestorePrevious
            } else {
                GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint
            };
            self.request_media_match_background_worker_cancel(
                handle,
                projected_state,
                disposition,
                "canceling broad Media Matching work for exact playlist fingerprint",
            );
            return true;
        } else if !force_restart
            && self.media_match_background_trigger_key.as_deref() == Some(trigger_key.as_str())
        {
            return true;
        }

        let tool_snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        if tool_snapshot.health != GuiMediaMatchToolHealth::Healthy {
            if notify_on_finish {
                let message = tool_snapshot.message.unwrap_or_else(|| {
                    "Media Matching tools are not ready for playlist fingerprint sharing."
                        .to_owned()
                });
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return false;
        }

        let settings = projected_state.media_match.settings.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let worker_cancel_flag = Arc::clone(&cancel_flag);
        let (tx, rx) = mpsc::channel();
        let backup_existed = match prepare_media_match_index_rebuild_backup(&root) {
            Ok(backup_existed) => backup_existed,
            Err(error) => {
                if notify_on_finish {
                    Self::push_runtime_error_notification(handle, projected_state, error);
                }
                return false;
            }
        };
        let backup_root = root.clone();
        let worker_path = path.clone();

        match thread::Builder::new()
            .name("sorotte-gui-media-match-exact-signature".to_owned())
            .spawn(move || {
                let progress_tx = tx.clone();
                let extraction_settings = media_match_sampled_fast_extraction_settings();
                let result = media_match_tool_paths_for_settings(&root, &extraction_settings)
                    .and_then(|tools| {
                        rebuild_persisted_media_match_candidates_with_progress_and_cancel(
                            MediaMatchCandidateRebuildRequest {
                                root: &root,
                                candidates: vec![PathBuf::from(&worker_path)],
                                current_player_path: Some(worker_path.as_str()),
                                settings: &settings,
                                tools: &tools,
                                extraction_settings: &extraction_settings,
                                cancel_flag: Some(worker_cancel_flag.as_ref()),
                            },
                            |progress| {
                                let _ = progress_tx
                                    .send(GuiMediaMatchBackgroundWorkerEvent::Progress(progress));
                            },
                        )
                    });
                let result = result.map(|mut result| {
                    result.message = format!(
                        "Media Matching playlist fingerprint ready. {}",
                        result.message
                    );
                    result
                });
                let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(result));
            }) {
            Ok(_thread) => {
                self.media_match_background_worker_rx = Some(rx);
                self.media_match_background_worker_cancel = Some(cancel_flag);
                self.media_match_background_trigger_key = Some(trigger_key);
                self.media_match_background_index_backup = Some(GuiMediaMatchIndexRebuildBackup {
                    root: backup_root.clone(),
                    backup_existed,
                });
                self.media_match_background_cancel_disposition = None;
                self.publish_media_match_background_status(
                    handle,
                    projected_state,
                    "queued: exact playlist fingerprint sharing",
                );
                true
            }
            Err(error) => {
                self.media_match_background_worker_cancel = None;
                self.media_match_background_worker_rx = None;
                let _ = discard_media_match_index_rebuild_backup(&backup_root);
                if notify_on_finish {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!(
                            "Could not start Media Matching playlist fingerprint worker: {error}"
                        ),
                    );
                }
                false
            }
        }
    }

    fn media_match_remote_targets_for_state(
        &self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Vec<GuiMediaMatchRemoteTarget> {
        let playlist_target = self
            .current_shared_playlist_target(projected_state)
            .and_then(|target| normalized_editable_text(&target));
        self.session
            .as_ref()
            .map(|session| session.current_room_media_match_peer_file_states())
            .unwrap_or_default()
            .into_iter()
            .filter(|peer| peer.has_file)
            .filter_map(|peer| {
                let media_match_signature = peer.media_match_signature?;
                let target_file_name = playlist_target
                    .clone()
                    .or_else(|| Self::usable_media_match_peer_file_name(peer.file_name))?;
                Some(GuiMediaMatchRemoteTarget {
                    target_file_name,
                    media_match_signature,
                })
            })
            .collect()
    }

    pub(in crate::app::runtime_owner) fn media_match_remote_resolution_token_for_state(
        &self,
        projected_state: &SorotteGuiShellAppState,
    ) -> String {
        if !projected_state.media_match.settings.fingerprinting_enabled {
            return String::new();
        }

        let mut targets = self
            .media_match_remote_targets_for_state(projected_state)
            .into_iter()
            .map(|target| {
                format!(
                    "{}\t{}",
                    target.target_file_name,
                    serde_json::to_string(&target.media_match_signature).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        targets.sort();
        targets.join("\n")
    }

    fn media_match_preferred_remote_target_for_state(
        &self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<GuiMediaMatchRemoteTarget> {
        let room_target = self.media_match_room_target_for_state(projected_state);
        let mut targets = self.media_match_remote_targets_for_state(projected_state);
        if let Some(room_target) = room_target.as_deref()
            && let Some(index) = targets.iter().position(|target| {
                target.target_file_name.eq_ignore_ascii_case(room_target)
                    || Path::new(&target.target_file_name)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case(room_target))
            })
        {
            return Some(targets.remove(index));
        }
        if room_target.is_some() {
            return None;
        }
        targets.into_iter().next()
    }

    fn media_match_remote_target_for_target(
        &self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<GuiMediaMatchRemoteTarget> {
        let room_target = Path::new(target)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(target);
        self.media_match_remote_targets_for_state(projected_state)
            .into_iter()
            .find(|remote| {
                remote.target_file_name.eq_ignore_ascii_case(target)
                    || Path::new(&remote.target_file_name)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case(room_target))
            })
    }

    fn media_match_remote_lookup_trigger_key(
        root: &Path,
        search_roots: &[PathBuf],
        candidate_paths: Option<&[PathBuf]>,
        remote: &GuiMediaMatchRemoteTarget,
        settings: &sorotte_media_match::MediaMatchSettings,
    ) -> String {
        let roots = search_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let candidate_source = Self::media_match_remote_lookup_candidate_source(candidate_paths);
        let signature = serde_json::to_string(&remote.media_match_signature).unwrap_or_default();
        format!(
            "root={}\nroots={roots}\ncandidates={candidate_source}\ntarget={}\nsignature={}\nsettings={settings:?}",
            root.display(),
            remote.target_file_name,
            signature
        )
    }

    fn media_match_remote_lookup_candidate_source(candidate_paths: Option<&[PathBuf]>) -> String {
        let Some(candidate_paths) = candidate_paths else {
            return "search-roots".to_owned();
        };
        let mut hasher = DefaultHasher::new();
        for path in candidate_paths {
            let mut key = path.to_string_lossy().replace('\\', "/");
            if cfg!(windows) {
                key = key.to_ascii_lowercase();
            }
            key.hash(&mut hasher);
        }
        format!("indexed:{}:{:016x}", candidate_paths.len(), hasher.finish())
    }

    fn cached_media_match_remote_lookup_result(&self, trigger_key: &str) -> Option<Option<String>> {
        self.media_match_remote_lookup_result
            .as_ref()
            .filter(|result| result.trigger_key == trigger_key)
            .map(|result| result.candidate_path.clone())
    }

    fn cancel_attached_media_search_after_media_match_resolution(&mut self) {
        self.unresolved_attached_media_target = None;
        self.attached_media_search_next_retry_at = None;
        if self.pending_attached_media_resolution.is_some() {
            self.cancel_pending_attached_media_search_index_build_impl();
        }
    }

    fn queue_media_match_remote_lookup_worker(
        &mut self,
        trigger_key: String,
        root: PathBuf,
        search_roots: Vec<PathBuf>,
        candidate_paths: Option<Vec<PathBuf>>,
        remote: GuiMediaMatchRemoteTarget,
        settings: sorotte_media_match::MediaMatchSettings,
    ) {
        if self
            .media_match_remote_lookup_trigger_key
            .as_deref()
            .is_some_and(|current| current == trigger_key)
        {
            return;
        }
        if self.media_match_remote_lookup_rx.is_some() {
            self.media_match_remote_lookup_rx = None;
            self.media_match_remote_lookup_trigger_key = None;
        }

        let worker_trigger_key = trigger_key.clone();
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("sorotte-gui-media-match-remote-lookup".to_owned())
            .spawn(move || {
                let extraction_settings = media_match_sampled_fast_extraction_settings();
                let candidate = media_match_cached_probable_candidate_for_remote_signature(
                    &root,
                    &search_roots,
                    candidate_paths.as_deref(),
                    &remote.target_file_name,
                    &remote.media_match_signature,
                    &settings,
                    &extraction_settings,
                )
                .map(|candidate| candidate.path);
                let _ = tx.send(GuiMediaMatchRemoteLookupResult {
                    trigger_key: worker_trigger_key,
                    candidate_path: candidate,
                });
            }) {
            Ok(_thread) => {
                self.media_match_remote_lookup_rx = Some(rx);
                self.media_match_remote_lookup_trigger_key = Some(trigger_key);
            }
            Err(_) => {
                self.media_match_remote_lookup_result = Some(GuiMediaMatchRemoteLookupResult {
                    trigger_key,
                    candidate_path: None,
                });
            }
        }
    }

    pub(in crate::app::runtime_owner) fn pump_media_match_remote_lookup_worker(&mut self) -> bool {
        let Some(rx) = self.media_match_remote_lookup_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                if self.media_match_remote_lookup_trigger_key.as_deref()
                    == Some(result.trigger_key.as_str())
                {
                    self.media_match_remote_lookup_trigger_key = None;
                    self.media_match_remote_lookup_result = Some(result);
                    self.media_match_wire_sync_token = None;
                    self.last_attached_media_resolution_trigger = None;
                    return true;
                }
                false
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.media_match_remote_lookup_rx = Some(rx);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(trigger_key) = self.media_match_remote_lookup_trigger_key.take() {
                    self.media_match_remote_lookup_result = Some(GuiMediaMatchRemoteLookupResult {
                        trigger_key,
                        candidate_path: None,
                    });
                    self.media_match_wire_sync_token = None;
                    self.last_attached_media_resolution_trigger = None;
                    return true;
                }
                false
            }
        }
    }

    fn media_match_remote_lookup_request_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<GuiMediaMatchRemoteLookupRequest> {
        if !Self::media_match_resolution_enabled(projected_state) {
            return None;
        }
        let root = self.media_match_root_for_request(projected_state)?;
        let search_roots = self.automatic_media_search_roots(projected_state);
        if search_roots.is_empty() {
            return None;
        }
        let remote = self.media_match_remote_target_for_target(projected_state, target)?;
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let candidate_paths = self.attached_media_match_candidate_paths(&roots);
        let trigger_key = Self::media_match_remote_lookup_trigger_key(
            &root,
            &search_roots,
            candidate_paths.as_deref(),
            &remote,
            &projected_state.media_match.settings,
        );
        Some(GuiMediaMatchRemoteLookupRequest {
            root,
            search_roots,
            candidate_paths,
            remote,
            trigger_key,
        })
    }

    pub(in crate::app::runtime_owner) fn media_match_cached_room_candidate_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<String> {
        let lookup = self.media_match_remote_lookup_request_for_target(projected_state, target)?;
        if let Some(candidate_path) =
            self.cached_media_match_remote_lookup_result(&lookup.trigger_key)
        {
            if candidate_path.is_some() {
                self.cancel_attached_media_search_after_media_match_resolution();
            }
            return candidate_path;
        }
        self.queue_media_match_remote_lookup_worker(
            lookup.trigger_key,
            lookup.root,
            lookup.search_roots,
            lookup.candidate_paths,
            lookup.remote,
            projected_state.media_match.settings.clone(),
        );
        None
    }

    pub(in crate::app::runtime_owner) fn media_match_remote_lookup_pending_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
    ) -> bool {
        let Some(lookup) =
            self.media_match_remote_lookup_request_for_target(projected_state, target)
        else {
            return false;
        };
        self.media_match_remote_lookup_rx.is_some()
            && self.media_match_remote_lookup_trigger_key.as_deref()
                == Some(lookup.trigger_key.as_str())
    }

    pub(in crate::app::runtime_owner) fn media_match_cached_exact_inventory_candidate_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
        search_roots: &[PathBuf],
    ) -> Option<String> {
        if !Self::media_match_resolution_enabled(projected_state) {
            return None;
        }
        let root = self.media_match_root_for_request(projected_state)?;
        let targets = Self::local_media_search_candidates_for_target(target);
        media_match_inventory_exact_candidate_for_targets(&root, search_roots, &targets)
    }

    fn current_player_path_if_cached_media_match_candidate_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
        current_path: &str,
    ) -> Option<String> {
        if !Self::media_match_resolution_enabled(projected_state) {
            return None;
        }
        let root = self.media_match_root_for_request(projected_state)?;
        let search_roots = self.automatic_media_search_roots(projected_state);
        if search_roots.is_empty() {
            return None;
        }
        let remote = self.media_match_remote_target_for_target(projected_state, target)?;
        let roots = Self::automatic_media_search_root_keys(&search_roots);
        let candidate_paths = self.attached_media_match_candidate_paths(&roots);
        let trigger_key = Self::media_match_remote_lookup_trigger_key(
            &root,
            &search_roots,
            candidate_paths.as_deref(),
            &remote,
            &projected_state.media_match.settings,
        );
        let candidate_path = self.cached_media_match_remote_lookup_result(&trigger_key)??;
        if Self::normalized_current_player_match_key(&candidate_path)
            != Self::normalized_current_player_match_key(current_path)
        {
            return None;
        }
        self.cancel_attached_media_search_after_media_match_resolution();
        Some(current_path.to_owned())
    }

    fn media_match_room_target_for_state(
        &self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<String> {
        self.current_shared_playlist_target(projected_state)
            .or_else(|| {
                self.session
                    .as_ref()
                    .and_then(|session| session.missing_media_search_target_file_name().ok())
            })
            .map(|target| target.trim().to_owned())
            .filter(|target| !target.is_empty())
    }

    fn media_match_current_local_path_for_state(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<String> {
        let room_target = self.media_match_room_target_for_state(projected_state);
        if let Some(path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.clone())
            .filter(|path| Path::new(path).is_file())
        {
            match room_target.as_deref() {
                None => return Some(path),
                Some(target) if self.current_player_matches_media_target(target) => {
                    return Some(path);
                }
                Some(target) => {
                    if let Some(path) = self
                        .current_player_path_if_cached_media_match_candidate_for_target(
                            projected_state,
                            target,
                            &path,
                        )
                    {
                        return Some(path);
                    }
                }
            }
        }

        let target = room_target?;
        match self.resolve_main_window_user_media_target(projected_state, &target) {
            Ok(GuiUserMediaTargetResolution::Resolved { path, .. })
                if Path::new(&path).is_file() =>
            {
                Some(path)
            }
            Ok(GuiUserMediaTargetResolution::Resolved { .. })
            | Ok(GuiUserMediaTargetResolution::Pending | GuiUserMediaTargetResolution::Missing)
            | Err(_) => None,
        }
    }

    fn media_match_wire_local_path_for_state(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> Option<String> {
        self.player_local_file
            .as_ref()
            .and_then(|file| file.path.clone())
            .filter(|path| Path::new(path).is_file())
            .or_else(|| self.media_match_current_local_path_for_state(projected_state))
    }

    fn attached_media_match_candidate_paths(&self, roots: &[String]) -> Option<Vec<PathBuf>> {
        let index = self
            .attached_media_search_index
            .as_ref()
            .filter(|index| index.roots == roots)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut paths = Vec::new();

        for root_key in roots {
            let Some(root_index) = index.root_indexes_by_key.get(root_key) else {
                continue;
            };
            for relative_paths in root_index.candidates_by_name.values() {
                for relative_path in relative_paths {
                    let candidate = if cfg!(windows) || !relative_path.contains('\\') {
                        root_index.root_path.join(relative_path)
                    } else {
                        root_index.root_path.join(relative_path.replace('\\', "/"))
                    };
                    let mut key = candidate.to_string_lossy().replace('\\', "/");
                    if cfg!(windows) {
                        key = key.to_ascii_lowercase();
                    }
                    if seen.insert(key) {
                        paths.push(candidate);
                    }
                }
            }
        }
        paths.sort_by(|left, right| {
            Self::normalized_current_player_match_key(&left.to_string_lossy())
                .cmp(&Self::normalized_current_player_match_key(
                    &right.to_string_lossy(),
                ))
                .then_with(|| left.cmp(right))
        });

        (!paths.is_empty()).then_some(paths)
    }

    fn queue_media_match_background_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        reason: &'static str,
        force_restart: bool,
        notify_on_finish: bool,
    ) -> bool {
        if !projected_state.media_match.settings.fingerprinting_enabled {
            if notify_on_finish {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    "Enable Media Matching fingerprinting before rebuilding the index.".to_owned(),
                );
            }
            return false;
        }
        if self.media_match_tool_worker_rx.is_some() {
            if notify_on_finish {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: "Media Matching tools are installing or importing; background matching will wait.".to_owned(),
                    }],
                );
            }
            return false;
        }
        let Some(root) = self.media_match_root_for_request(projected_state) else {
            if notify_on_finish {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    "Media Matching background work requires a writable GUI config root."
                        .to_owned(),
                );
            }
            return false;
        };
        match self.media_match_exact_playlist_plan_for_state(projected_state, &root) {
            GuiMediaMatchExactPlaylistPlan::None => {}
            GuiMediaMatchExactPlaylistPlan::ExactNoFingerprint { .. } => {
                if self.media_match_background_worker_rx.is_some() {
                    self.request_media_match_background_worker_cancel(
                        handle,
                        projected_state,
                        GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                        "idle: exact shared-playlist file already loaded",
                    );
                    return true;
                }
                self.media_match_background_trigger_key =
                    Some("exact-playlist-no-fingerprint".to_owned());
                self.publish_media_match_background_status(
                    handle,
                    projected_state,
                    "idle: exact shared-playlist file already loaded",
                );
                return true;
            }
            GuiMediaMatchExactPlaylistPlan::ExactNeedsSignature { path } => {
                return self.queue_exact_playlist_signature_worker(
                    handle,
                    projected_state,
                    root,
                    path,
                    force_restart,
                    notify_on_finish,
                );
            }
        }
        let search_roots = self.automatic_media_search_roots(projected_state);
        if search_roots.is_empty() {
            if notify_on_finish {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: "Media Matching has no media-search roots to warm.".to_owned(),
                    }],
                );
            }
            return false;
        }
        let current_player_path = self.media_match_current_local_path_for_state(projected_state);
        let remote_candidate = current_player_path
            .is_none()
            .then(|| self.media_match_preferred_remote_target_for_state(projected_state))
            .flatten();
        let extraction_required = current_player_path.is_some() || remote_candidate.is_some();
        let tool_snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        if extraction_required && tool_snapshot.health != GuiMediaMatchToolHealth::Healthy {
            if notify_on_finish {
                let message = tool_snapshot.message.unwrap_or_else(|| {
                    "Media Matching tools are not ready for fingerprint extraction.".to_owned()
                });
                Self::push_runtime_error_notification(handle, projected_state, message);
            }
            return false;
        }

        let trigger_key = self.media_match_background_trigger_key(
            projected_state,
            &search_roots,
            current_player_path.as_deref(),
        );
        if self.media_match_background_worker_rx.is_some() {
            if !force_restart
                && self.media_match_background_trigger_key.as_deref() == Some(trigger_key.as_str())
            {
                return true;
            }
            let disposition = if force_restart {
                GuiMediaMatchBackgroundCancelDisposition::RestorePrevious
            } else {
                GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint
            };
            let status = if force_restart {
                "canceling current rebuild before restart"
            } else {
                "canceling current rebuild after input change"
            };
            self.request_media_match_background_worker_cancel(
                handle,
                projected_state,
                disposition,
                status,
            );
            return true;
        } else if !force_restart
            && self.media_match_background_trigger_key.as_deref() == Some(trigger_key.as_str())
        {
            return true;
        }

        let root_keys = Self::automatic_media_search_root_keys(&search_roots);
        let candidates = extraction_required
            .then(|| self.attached_media_match_candidate_paths(&root_keys))
            .flatten();
        let settings = projected_state.media_match.settings.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let worker_cancel_flag = Arc::clone(&cancel_flag);
        let (tx, rx) = mpsc::channel();
        let backup_existed = match prepare_media_match_index_rebuild_backup(&root) {
            Ok(backup_existed) => backup_existed,
            Err(error) => {
                if notify_on_finish {
                    Self::push_runtime_error_notification(handle, projected_state, error);
                }
                return false;
            }
        };
        let backup_root = root.clone();

        match thread::Builder::new()
            .name("sorotte-gui-media-match-background".to_owned())
            .spawn(move || {
                let progress_tx = tx.clone();
                let fast_result = if current_player_path.is_none() {
                    if let Some(remote_candidate) = remote_candidate.clone() {
                        let extraction_settings =
                            sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
                        media_match_tool_paths_for_settings(&root, &extraction_settings).and_then(|tools| {
                            let request = MediaMatchRemoteCandidateRebuildRequest {
                                root: &root,
                                search_roots: &search_roots,
                                candidates: candidates.clone(),
                                target_file_name: &remote_candidate.target_file_name,
                                media_match_signature: &remote_candidate.media_match_signature,
                                settings: &settings,
                                tools: &tools,
                                extraction_settings: &extraction_settings,
                                cancel_flag: Some(worker_cancel_flag.as_ref()),
                            };
                            rebuild_persisted_media_match_remote_candidates_with_progress_and_cancel(
                                request,
                                |progress| {
                                    let _ = progress_tx.send(
                                        GuiMediaMatchBackgroundWorkerEvent::Progress(progress),
                                    );
                                },
                            )
                        })
                    } else {
                        let extraction_settings =
                            sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
                        rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
                            &root,
                            &search_roots,
                            None,
                            &settings,
                            &extraction_settings,
                            Some(worker_cancel_flag.as_ref()),
                            |progress| {
                                let _ = progress_tx
                                    .send(GuiMediaMatchBackgroundWorkerEvent::Progress(progress));
                            },
                        )
                    }
                } else if let Some(candidates) = candidates.clone() {
                    let extraction_settings =
                        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
                    media_match_tool_paths_for_settings(&root, &extraction_settings).and_then(|tools| {
                        rebuild_persisted_media_match_candidates_with_progress_and_cancel(
                            MediaMatchCandidateRebuildRequest {
                                root: &root,
                                candidates,
                                current_player_path: current_player_path.as_deref(),
                                settings: &settings,
                                tools: &tools,
                                extraction_settings: &extraction_settings,
                                cancel_flag: Some(worker_cancel_flag.as_ref()),
                            },
                            |progress| {
                                let _ = progress_tx
                                    .send(GuiMediaMatchBackgroundWorkerEvent::Progress(progress));
                            },
                        )
                    })
                } else {
                    let extraction_settings =
                        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
                    rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
                        &root,
                        &search_roots,
                        current_player_path.as_deref(),
                        &settings,
                        &extraction_settings,
                        Some(worker_cancel_flag.as_ref()),
                        |progress| {
                            let _ = progress_tx
                                .send(GuiMediaMatchBackgroundWorkerEvent::Progress(progress));
                        },
                    )
                };
                let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(fast_result));
            }) {
            Ok(_thread) => {
                self.media_match_background_worker_rx = Some(rx);
                self.media_match_background_worker_cancel = Some(cancel_flag);
                self.media_match_background_trigger_key = Some(trigger_key);
                self.media_match_background_index_backup = Some(GuiMediaMatchIndexRebuildBackup {
                    root: backup_root.clone(),
                    backup_existed,
                });
                self.media_match_background_cancel_disposition = None;
                self.publish_media_match_background_status(
                    handle,
                    projected_state,
                    format!("queued: {reason}"),
                );
                true
            }
            Err(error) => {
                self.media_match_background_worker_cancel = None;
                self.media_match_background_worker_rx = None;
                let _ = discard_media_match_index_rebuild_backup(&backup_root);
                if notify_on_finish {
                    Self::push_runtime_error_notification(
                        handle,
                        projected_state,
                        format!("Could not start Media Matching background worker: {error}"),
                    );
                }
                false
            }
        }
    }

    fn apply_media_match_background_result(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        result: MediaMatchIndexRebuildResult,
        notify: bool,
        background_status: impl Into<String>,
    ) -> bool {
        let mut snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        snapshot.cache_status = Some(result.cache_status);
        snapshot.current_decision = result.current_decision;
        snapshot.nearest_match = result.nearest_match;
        snapshot.last_evidence = result.last_evidence.or_else(|| {
            Some(
                "Fingerprint evidence is local; optional raw wire signatures are shared only with room peers."
                    .to_owned(),
            )
        });
        snapshot.background_status = Some(background_status.into());
        self.media_match_runtime_snapshot = snapshot.clone();
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
        self.media_match_wire_sync_token = None;
        self.last_attached_media_resolution_trigger = None;
        self.clear_media_match_remote_lookup_state();
        if !self.sync_media_match_wire_decisions(handle, projected_state) {
            return false;
        }
        let snapshot = self.media_match_runtime_snapshot.clone();
        let mut actions = vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)];
        if notify {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: result.message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(result.message));
        }
        Self::push_actions_and_project(handle, projected_state, actions);
        true
    }

    pub(in crate::app::runtime_owner) fn pump_media_match_background_worker(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let Some(rx) = self.media_match_background_worker_rx.take() else {
            return;
        };
        let mut keep_rx = true;
        let mut processed_events = 0usize;
        let mut latest_progress = None;
        let plugin_enabled = projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching);
        loop {
            if processed_events >= MEDIA_MATCH_BACKGROUND_EVENTS_PER_PUMP {
                break;
            }
            match rx.try_recv() {
                Ok(GuiMediaMatchBackgroundWorkerEvent::Progress(progress)) => {
                    processed_events += 1;
                    if plugin_enabled {
                        latest_progress = Some(progress);
                    }
                }
                Ok(GuiMediaMatchBackgroundWorkerEvent::Finished(result)) => {
                    latest_progress = None;
                    keep_rx = false;
                    self.media_match_background_worker_cancel = None;
                    let cancel_disposition = self.media_match_background_cancel_disposition.take();
                    if !plugin_enabled {
                        if let Some(disposition) = cancel_disposition {
                            self.publish_media_match_background_cancel_status(
                                handle,
                                projected_state,
                                disposition,
                            );
                        }
                        break;
                    }
                    match result {
                        Ok(result) => {
                            if let Some(disposition) = cancel_disposition {
                                self.publish_media_match_background_cancel_status(
                                    handle,
                                    projected_state,
                                    disposition,
                                );
                                break;
                            }
                            let backup_warning =
                                self.finish_media_match_background_index_backup(true).err();
                            let background_status = if result.current_decision.as_deref()
                                == Some("unknown: no resolved current local file")
                            {
                                "idle: waiting for resolved local media"
                            } else {
                                "idle"
                            };
                            if !self.apply_media_match_background_result(
                                handle,
                                projected_state,
                                result,
                                true,
                                background_status,
                            ) {
                                break;
                            }
                            if let Some(error) = backup_warning {
                                Self::push_runtime_error_notification(
                                    handle,
                                    projected_state,
                                    error,
                                );
                            }
                        }
                        Err(error) if error.contains("canceled") => {
                            self.publish_media_match_background_cancel_status(
                                handle,
                                projected_state,
                                cancel_disposition.unwrap_or(
                                    GuiMediaMatchBackgroundCancelDisposition::RestorePrevious,
                                ),
                            );
                        }
                        Err(error) => {
                            if let Some(disposition) = cancel_disposition {
                                self.publish_media_match_background_cancel_status(
                                    handle,
                                    projected_state,
                                    disposition,
                                );
                                break;
                            }
                            let restore_error =
                                self.finish_media_match_background_index_backup(false).err();
                            let mut snapshot = self.refresh_media_match_runtime_snapshot(
                                &projected_state.media_match.settings,
                            );
                            let message = restore_error
                                .as_ref()
                                .map(|restore_error| {
                                    format!(
                                        "{error}; failed restoring previous index: {restore_error}"
                                    )
                                })
                                .unwrap_or_else(|| format!("{error}; previous index restored"));
                            snapshot.message = Some(message.clone());
                            snapshot.background_status = Some("failed".to_owned());
                            self.media_match_runtime_snapshot = snapshot.clone();
                            Self::push_actions_and_project(
                                handle,
                                projected_state,
                                vec![
                                    GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot),
                                    GuiShellAction::PushTransientNotification {
                                        level: GuiTransientNotificationLevel::Warning,
                                        message,
                                    },
                                ],
                            );
                        }
                    }
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    latest_progress = None;
                    keep_rx = false;
                    self.media_match_background_worker_cancel = None;
                    self.media_match_background_cancel_disposition = None;
                    if !plugin_enabled {
                        break;
                    }
                    let status = self
                        .finish_media_match_background_index_backup(false)
                        .map(|()| "failed: previous index restored".to_owned())
                        .unwrap_or_else(|error| format!("failed: restore failed: {error}"));
                    let mut snapshot = self.refresh_media_match_runtime_snapshot(
                        &projected_state.media_match.settings,
                    );
                    snapshot.background_status = Some(status);
                    self.media_match_runtime_snapshot = snapshot.clone();
                    Self::push_actions_and_project(
                        handle,
                        projected_state,
                        vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)],
                    );
                    break;
                }
            }
        }
        if let Some(progress) = latest_progress {
            self.publish_media_match_background_status(
                handle,
                projected_state,
                Self::media_match_background_progress_status(&progress),
            );
        }
        if keep_rx {
            self.media_match_background_worker_rx = Some(rx);
        }
    }

    pub(in crate::app::runtime_owner) fn maybe_queue_media_match_background_warmup(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            return;
        }
        if !projected_state.media_match.settings.fingerprinting_enabled {
            return;
        }
        if !projected_state
            .media_match
            .settings
            .background_warmup_enabled
        {
            return;
        }
        let waiting_for_room_resolution =
            self.media_match_background_warmup_should_wait_for_room_resolution(projected_state);
        if self.media_match_background_worker_rx.is_some() {
            if waiting_for_room_resolution {
                self.request_media_match_background_worker_cancel(
                    handle,
                    projected_state,
                    GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                    "canceling background warmup: waiting for resolved local media",
                );
            }
            return;
        }
        if waiting_for_room_resolution {
            self.publish_media_match_background_status(
                handle,
                projected_state,
                "idle: waiting for resolved local media",
            );
            return;
        }
        let _ = self.queue_media_match_background_worker(
            handle,
            projected_state,
            "background warmup",
            false,
            false,
        );
    }

    fn media_match_background_warmup_should_wait_for_room_resolution(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
    ) -> bool {
        self.media_match_room_target_for_state(projected_state)
            .is_some()
            && self
                .media_match_current_local_path_for_state(projected_state)
                .is_none()
            && self
                .media_match_preferred_remote_target_for_state(projected_state)
                .is_none()
    }

    pub(in crate::app::runtime_owner) fn maybe_queue_media_match_exact_playlist_signature(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            return;
        }
        if !projected_state.media_match.settings.fingerprinting_enabled
            || !projected_state.media_match.settings.wire_sharing_enabled
        {
            return;
        }
        let Some(root) = self.media_match_root_for_request(projected_state) else {
            return;
        };
        let GuiMediaMatchExactPlaylistPlan::ExactNeedsSignature { path } =
            self.media_match_exact_playlist_plan_for_state(projected_state, &root)
        else {
            return;
        };
        let _ = self.queue_exact_playlist_signature_worker(
            handle,
            projected_state,
            root,
            path,
            false,
            false,
        );
    }

    pub(super) fn handle_install_media_match_tools_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::MediaMatching,
            );
            return true;
        }
        if self.media_match_tool_worker_busy_notification(handle, projected_state) {
            return true;
        }
        let Some(root) = self.media_match_root_for_request(projected_state) else {
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
                "Installing ffmpeg and ffprobe into Sorotte's managed tools directory.".to_owned(),
            ),
            0.02,
        );
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("sorotte-gui-media-match-install".to_owned())
            .spawn(move || {
                let progress_tx = tx.clone();
                let result =
                    install_or_update_managed_media_match_tools_with_progress(&root, |progress| {
                        let _ = progress_tx.send(GuiMediaMatchToolWorkerEvent::Progress(progress));
                    });
                let _ = tx.send(GuiMediaMatchToolWorkerEvent::Finished {
                    result,
                    failure_label: "Media Matching tool install failed",
                });
            }) {
            Ok(_thread) => {
                self.media_match_tool_worker_rx = Some(rx);
            }
            Err(error) => self.finish_media_match_tool_error(
                handle,
                projected_state,
                "Media Matching tool install failed",
                format!("could not start worker: {error}"),
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
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::MediaMatching,
            );
            return true;
        }
        if self.media_match_tool_worker_busy_notification(handle, projected_state) {
            return true;
        }
        let Some(root) = self.media_match_root_for_request(projected_state) else {
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
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name(format!(
                "sorotte-gui-media-match-import-{}",
                tool.display_name()
            ))
            .spawn(move || {
                let progress_tx = tx.clone();
                let result = import_managed_media_match_tool_with_progress(
                    &root,
                    tool,
                    Path::new(&source_path),
                    |progress| {
                        let _ = progress_tx.send(GuiMediaMatchToolWorkerEvent::Progress(progress));
                    },
                );
                let _ = tx.send(GuiMediaMatchToolWorkerEvent::Finished {
                    result,
                    failure_label: "Media Matching tool import failed",
                });
            }) {
            Ok(_thread) => {
                self.media_match_tool_worker_rx = Some(rx);
            }
            Err(error) => self.finish_media_match_tool_error(
                handle,
                projected_state,
                "Media Matching tool import failed",
                format!("could not start worker: {error}"),
            ),
        }
        true
    }

    pub(super) fn handle_open_media_match_install_location_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        let Some(root) = self.media_match_root_for_request(projected_state) else {
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
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::MediaMatching,
            );
            return true;
        }
        let _ = self.media_match_config_path_for_request(projected_state);
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
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::MediaMatching,
            );
            return true;
        }
        let _ = self.queue_media_match_background_worker(
            handle,
            projected_state,
            "manual rebuild",
            true,
            true,
        );
        true
    }

    pub(super) fn handle_cancel_media_match_rebuild_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if self.request_media_match_background_worker_cancel(
            handle,
            projected_state,
            GuiMediaMatchBackgroundCancelDisposition::RestorePrevious,
            "canceling rebuild: restoring previous index",
        ) {
            return true;
        }
        Self::push_actions_and_project(
            handle,
            projected_state,
            vec![GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "No Media Matching rebuild is running.".to_owned(),
            }],
        );
        true
    }

    pub(super) fn handle_clear_media_match_cache_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !projected_state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Self::push_plugin_disabled_notification(
                handle,
                projected_state,
                GuiPluginSelection::MediaMatching,
            );
            return true;
        }
        if self.request_media_match_background_worker_cancel(
            handle,
            projected_state,
            GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
            "canceling background rebuild before clearing cache",
        ) {
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Canceling Media Matching background work before clearing cache. Run Clear Match Cache again when it is idle.".to_owned(),
                }],
            );
            return true;
        }
        if let Some(root) = self.media_match_root_for_request(projected_state)
            && let Err(error) = clear_persisted_media_match_cache_at_root(&root)
        {
            Self::push_runtime_error_notification(handle, projected_state, error);
            return false;
        }
        let mut snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        snapshot.cache_status = Some("empty".to_owned());
        snapshot.current_decision = None;
        snapshot.nearest_match = None;
        snapshot.last_evidence = None;
        snapshot.background_status = Some("idle".to_owned());
        self.media_match_runtime_snapshot = snapshot.clone();
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
        self.media_match_wire_sync_token = None;
        if !self.sync_media_match_wire_decisions(handle, projected_state) {
            return true;
        }
        let snapshot = self.media_match_runtime_snapshot.clone();
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
        let was_enabled = projected_state.media_match.settings.fingerprinting_enabled;
        let should_start_initial_index = enabled
            && !was_enabled
            && projected_state
                .plugin_enablement
                .enabled_for(GuiPluginSelection::MediaMatching)
            && self
                .media_match_root_for_request(projected_state)
                .is_some_and(|root| !media_match_sqlite_index_exists(&root));
        if !enabled {
            self.request_media_match_background_worker_cancel(
                handle,
                projected_state,
                GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                "canceling background rebuild: fingerprinting disabled",
            );
        }
        projected_state.media_match.settings.fingerprinting_enabled = enabled;
        if !self.persist_media_match_settings_request(handle, projected_state) {
            return false;
        }
        if should_start_initial_index {
            let _ = self.queue_media_match_background_worker(
                handle,
                projected_state,
                "initial index build",
                false,
                true,
            );
        }
        true
    }

    pub(super) fn handle_set_media_match_background_warmup_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        projected_state
            .media_match
            .settings
            .background_warmup_enabled = enabled;
        if !enabled {
            self.request_media_match_background_worker_cancel(
                handle,
                projected_state,
                GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                "canceling background warmup",
            );
        }
        self.persist_media_match_settings_request(handle, projected_state)
    }

    pub(super) fn handle_set_media_match_wire_sharing_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        enabled: bool,
    ) -> bool {
        projected_state.media_match.settings.wire_sharing_enabled = enabled;
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
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
        let Some(config_path) = self.media_match_config_path_for_request(projected_state) else {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Could not persist Media Matching settings: no writable GUI config path is available."
                    .to_owned(),
            );
            return false;
        };
        if let Err(error) = upsert_sorotte_ini_stored_client_settings_mvp_at_path(
            &config_path,
            &projected_state.configuration.settings,
        ) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Could not persist Media Matching settings: {error}"),
            );
            return false;
        }
        let snapshot =
            self.refresh_media_match_runtime_snapshot(&projected_state.media_match.settings);
        self.media_match_runtime_snapshot = snapshot.clone();
        self.last_published_local_file = None;
        self.last_published_media_match_signature = None;
        self.media_match_wire_sync_token = None;
        if !self.sync_media_match_wire_decisions(handle, projected_state) {
            return false;
        }
        let snapshot = self.media_match_runtime_snapshot.clone();
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
        self.maybe_queue_media_match_background_warmup(handle, projected_state);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime_owner::{
        GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
        GuiAttachedMediaSearchRootIndex, GuiPendingAttachedMediaResolution,
    };
    use crate::app::runtime_stack::GuiSessionRuntimeAdapter;
    use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

    struct MediaMatchTargetSession {
        target: String,
    }

    impl GuiSessionRuntimeAdapter for MediaMatchTargetSession {
        fn missing_media_search_target_file_name(&self) -> Result<String, String> {
            Ok(self.target.clone())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(current_servers)
        }
    }

    #[derive(Debug, Clone)]
    struct MediaMatchPeerStateSession {
        peer_files: Vec<sorotte_client_core::ClientMediaMatchPeerFileState>,
    }

    impl GuiSessionRuntimeAdapter for MediaMatchPeerStateSession {
        fn current_room_media_match_peer_file_states(
            &self,
        ) -> Vec<sorotte_client_core::ClientMediaMatchPeerFileState> {
            self.peer_files.clone()
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(current_servers)
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    fn media_match_test_record_for_path(
        path: impl AsRef<std::path::Path>,
        anchor_seed: u32,
    ) -> sorotte_media_match::MediaFingerprintRecord {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path).expect("test media should exist");
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(
                path,
                modified_unix_millis,
                metadata.len(),
            ),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{}", path.display()),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        record.audio_anchors = (0u32..24)
            .map(|index| sorotte_media_match::AudioAnchor {
                bucket: anchor_seed + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
        record
    }

    fn remote_media_match_test_record(
        path: &str,
        anchor_seed: u32,
    ) -> sorotte_media_match::MediaFingerprintRecord {
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(path, 1000, 2000),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{path}"),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        record.audio_anchors = (0u32..24)
            .map(|index| sorotte_media_match::AudioAnchor {
                bucket: anchor_seed + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
        record
    }

    fn wait_for_media_match_remote_lookup(owner: &mut GuiPersistedConfigRuntimeOwner) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            owner.pump_media_match_remote_lookup_worker();
            if owner.media_match_remote_lookup_rx.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out waiting for cached media-match remote lookup completion");
    }

    #[test]
    fn background_worker_with_attached_index_and_no_current_file_inventories_only() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-inventory-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_root = root.join("media");
        std::fs::create_dir_all(&media_root).expect("media root should be created");
        std::fs::write(media_root.join("episode.mkv"), b"not real media")
            .expect("candidate file should be created");
        let media_root_text = media_root.to_string_lossy().into_owned();
        let saved_settings = StoredClientSettingsMvp {
            media_search_directories: Some(vec![media_root_text]),
            media_match_fingerprinting_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        let root_key =
            crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
        owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
            roots: vec![root_key.clone()],
            root_indexes_by_key: std::collections::HashMap::from([(
                root_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: root_key.clone(),
                    root_path: media_root.clone(),
                    built_at_unix_ms: 1,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode.mkv".to_owned(),
                        vec!["episode.mkv".to_owned()],
                    )]),
                },
            )]),
            roots_requiring_refresh: std::collections::BTreeSet::new(),
        });
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert!(owner.queue_media_match_background_worker(
            &handle,
            &mut state,
            "test inventory",
            true,
            false,
        ));
        let rx = owner
            .media_match_background_worker_rx
            .take()
            .expect("worker should be queued");
        let result = loop {
            match rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("worker should finish")
            {
                GuiMediaMatchBackgroundWorkerEvent::Progress(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::Finished(result) => break result,
            }
        }
        .expect("inventory-only worker should succeed without media tools");
        owner
            .finish_media_match_background_index_backup(true)
            .expect("backup should be discarded");

        let summary =
            sorotte_media_match::MediaIndexService::new(root.join("cache").join("media-match"))
                .open()
                .expect("media index should open")
                .summary(
                    &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
                )
                .expect("media index summary should load");

        assert_eq!(summary.inventory_count, 1);
        assert_eq!(summary.v3_fingerprint_row_count, 0);
        assert_eq!(
            result.current_decision,
            Some("unknown: no resolved current local file".to_owned())
        );
        assert_eq!(result.nearest_match, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_current_path_resolves_room_target_file_without_player_update() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-room-target-current-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_root = root.join("media");
        let nested_directory = media_root.join("Bakemonogatari");
        std::fs::create_dir_all(&nested_directory).expect("media root should be created");
        let media_path = nested_directory.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
            media_match_fingerprinting_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        let root_key =
            crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_session_runtime(Box::new(MediaMatchTargetSession {
                target: "episode.mkv".to_owned(),
            }));
        owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
            roots: vec![root_key.clone()],
            root_indexes_by_key: std::collections::HashMap::from([(
                root_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: root_key.clone(),
                    root_path: media_root.clone(),
                    built_at_unix_ms: 1,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode.mkv".to_owned(),
                        vec!["Bakemonogatari\\episode.mkv".to_owned()],
                    )]),
                },
            )]),
            roots_requiring_refresh: std::collections::BTreeSet::new(),
        });

        assert!(owner.player_local_file.is_none());
        assert_eq!(
            owner.media_match_room_target_for_state(&state),
            Some("episode.mkv".to_owned())
        );
        assert_eq!(
            owner.media_match_current_local_path_for_state(&state),
            Some(media_path.to_string_lossy().into_owned())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_current_path_resolves_active_shared_playlist_file_without_player_update() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-playlist-current-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_root = root.join("media");
        std::fs::create_dir_all(&media_root).expect("media root should be created");
        let media_path = media_root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
            media_match_fingerprinting_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);

        assert!(owner.player_local_file.is_none());
        assert_eq!(
            owner.media_match_current_local_path_for_state(&state),
            Some(media_path.to_string_lossy().into_owned())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_current_path_ignores_previous_player_file_for_new_playlist_target() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-playlist-previous-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_root = root.join("media");
        std::fs::create_dir_all(&media_root).expect("media root should be created");
        let previous_media_path = media_root.join("episode1.mkv");
        std::fs::write(&previous_media_path, b"not real media")
            .expect("previous media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
            media_match_fingerprinting_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(
            vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
            Some(1),
            false,
        );
        let root_key =
            crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(1);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_path(previous_media_path.to_string_lossy().into_owned()),
        );
        owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
            roots: vec![root_key.clone()],
            root_indexes_by_key: std::collections::HashMap::from([(
                root_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: root_key.clone(),
                    root_path: media_root.clone(),
                    built_at_unix_ms: 1,
                    candidates_by_name: std::collections::HashMap::new(),
                },
            )]),
            roots_requiring_refresh: std::collections::BTreeSet::new(),
        });

        assert_eq!(
            owner.media_match_room_target_for_state(&state).as_deref(),
            Some("episode2.mkv")
        );
        assert_eq!(owner.media_match_current_local_path_for_state(&state), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_wire_status_uses_open_alternate_encode_for_playlist_target() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-wire-alternate-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_root = root.join("media");
        std::fs::create_dir_all(&media_root).expect("media root should be created");
        let local_media_path = media_root.join("coalgirls-episode4.mkv");
        std::fs::write(&local_media_path, b"alternate local encode")
            .expect("local alternate fixture should be written");
        let remote_file_name = "mtbb-mini-episode4.mkv";

        let local_record = media_match_test_record_for_path(&local_media_path, 100);
        let mut cache = sorotte_media_match::MediaMatchCache::default();
        cache.insert(local_record);
        crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
            .expect("media-match cache should be written");
        let remote_signature = sorotte_media_match::media_match_wire_value_from_records(&[
            remote_media_match_test_record(remote_file_name, 100),
        ])
        .expect("remote media-match signature should serialize");

        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_session_runtime(Box::new(MediaMatchPeerStateSession {
                peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                    username: "bob".to_owned(),
                    has_file: true,
                    file_name: Some(remote_file_name.to_owned()),
                    file_size: None,
                    file_duration: None,
                    media_match_signature: Some(
                        sorotte_media_match::media_match_wire_signature_from_value(
                            &remote_signature,
                        )
                        .expect("remote signature should validate"),
                    ),
                }],
            }));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("coalgirls-episode4.mkv")
                .with_path(local_media_path.to_string_lossy().into_owned()),
        );
        owner.media_match_runtime_snapshot.health = crate::app::GuiMediaMatchToolHealth::Healthy;

        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        state.apply_shared_playlist_entries(vec![remote_file_name.to_owned()], Some(0), false);
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        let local_media_path_text = local_media_path.to_string_lossy().into_owned();
        assert_eq!(
            owner.current_player_path_if_cached_media_match_candidate_for_target(
                &state,
                remote_file_name,
                &local_media_path_text,
            ),
            None,
            "an alternate-encode player file should not count as current media before media matching resolves it"
        );
        assert!(owner.pending_attached_media_resolution.is_none());
        assert!(owner.attached_media_search_index.is_none());
        assert_eq!(
            owner.media_match_cached_room_candidate_for_target(&state, remote_file_name),
            None,
            "first probable-candidate lookup should run asynchronously"
        );
        wait_for_media_match_remote_lookup(&mut owner);
        let (_result_tx, result_rx) =
            std::sync::mpsc::channel::<GuiAttachedMediaSearchBuildStatus>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
            roots: vec![],
            cancel_flag: Arc::clone(&cancel_flag),
            latest_progress: Arc::new(std::sync::Mutex::new(None)),
            result_rx,
        });
        owner.attached_media_search_next_retry_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
        assert_eq!(
            owner.media_match_cached_room_candidate_for_target(&state, remote_file_name),
            Some(sorotte_media_match::normalize_media_path(&local_media_path))
        );
        assert!(
            cancel_flag.load(Ordering::Relaxed),
            "using a cached media-match candidate should cancel an obsolete attached-media scan"
        );
        assert!(owner.pending_attached_media_resolution.is_none());
        assert!(owner.attached_media_search_next_retry_at.is_none());
        assert_eq!(
            owner.media_match_current_local_path_for_state(&state),
            Some(local_media_path_text.clone()),
            "a cached probable media-match candidate should count as the current local file"
        );
        assert!(
            owner.pending_attached_media_resolution.is_none(),
            "using a cached media-match candidate as the current local file should not queue an attached media search"
        );
        assert!(
            owner.attached_media_search_index.is_none(),
            "using a cached media-match candidate as the current local file should not load the attached media-search index"
        );
        assert_eq!(
            owner.media_match_wire_local_path_for_state(&state),
            Some(local_media_path_text),
            "wire comparison should use the real open local file for alternate encodes"
        );

        assert!(owner.sync_media_match_wire_decisions(&handle, &mut state));
        assert_eq!(
            owner.media_match_runtime_snapshot.remote_status.as_deref(),
            Some("bob: probable")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_background_progress_backlog_yields_between_runtime_pumps() {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let (tx, rx) = mpsc::channel();

        for index in 0..(MEDIA_MATCH_BACKGROUND_EVENTS_PER_PUMP + 10) {
            tx.send(GuiMediaMatchBackgroundWorkerEvent::Progress(
                MediaMatchToolProgress {
                    label: "Fingerprinting media".to_owned(),
                    detail: Some(format!("{index} files needing index")),
                    progress_fraction: index as f32
                        / (MEDIA_MATCH_BACKGROUND_EVENTS_PER_PUMP + 10) as f32,
                },
            ))
            .expect("progress backlog should be queued");
        }

        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(Arc::new(AtomicBool::new(false)));
        owner.media_match_background_trigger_key = Some("progress-backlog".to_owned());

        owner.pump_media_match_background_worker(&handle, &mut state);

        let rx = owner
            .media_match_background_worker_rx
            .as_ref()
            .expect("background worker should remain pending after a partial progress drain");
        assert!(
            matches!(
                rx.try_recv(),
                Ok(GuiMediaMatchBackgroundWorkerEvent::Progress(_))
            ),
            "a single GUI runtime pump must not consume an unbounded background progress backlog"
        );
    }

    #[test]
    fn canceled_media_match_background_worker_ok_result_does_not_publish_stale_nearest_match() {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.media_match_runtime_snapshot.nearest_match = Some("current nearest".to_owned());
        owner.media_match_runtime_snapshot.last_evidence = Some("current evidence".to_owned());
        let (tx, rx) = mpsc::channel();
        tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(Ok(
            MediaMatchIndexRebuildResult {
                message: "stale result".to_owned(),
                cache_status: "stale cache".to_owned(),
                current_decision: Some("stale decision".to_owned()),
                nearest_match: Some("stale nearest".to_owned()),
                last_evidence: Some("stale evidence".to_owned()),
            },
        )))
        .expect("stale worker result should be queued");

        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(Arc::new(AtomicBool::new(true)));
        owner.media_match_background_cancel_disposition =
            Some(GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint);
        owner.media_match_background_trigger_key = Some("stale-trigger".to_owned());

        owner.pump_media_match_background_worker(&handle, &mut state);

        assert!(owner.media_match_background_worker_rx.is_none());
        assert!(owner.media_match_background_worker_cancel.is_none());
        assert_eq!(
            owner.media_match_runtime_snapshot.nearest_match.as_deref(),
            Some("current nearest"),
            "a worker result that arrives after cancellation must not publish stale nearest-match text"
        );
        assert_eq!(
            owner.media_match_runtime_snapshot.last_evidence.as_deref(),
            Some("current evidence"),
            "a worker result that arrives after cancellation must not publish stale evidence"
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("canceled: checkpoint kept")
        );
    }

    #[test]
    fn disabled_media_match_background_worker_result_does_not_publish_stale_nearest_match() {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            media_matching_plugin_enabled: Some(false),
            media_match_fingerprinting_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.media_match_runtime_snapshot.nearest_match = Some("current nearest".to_owned());
        owner.media_match_runtime_snapshot.last_evidence = Some("current evidence".to_owned());
        let (tx, rx) = mpsc::channel();
        tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(Ok(
            MediaMatchIndexRebuildResult {
                message: "stale result".to_owned(),
                cache_status: "stale cache".to_owned(),
                current_decision: Some("stale decision".to_owned()),
                nearest_match: Some("stale nearest".to_owned()),
                last_evidence: Some("stale evidence".to_owned()),
            },
        )))
        .expect("stale worker result should be queued");

        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(Arc::new(AtomicBool::new(false)));
        owner.media_match_background_cancel_disposition =
            Some(GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint);
        owner.media_match_background_trigger_key = Some("disabled-trigger".to_owned());

        owner.pump_media_match_background_worker(&handle, &mut state);

        assert!(owner.media_match_background_worker_rx.is_none());
        assert!(owner.media_match_background_worker_cancel.is_none());
        assert_eq!(
            owner.media_match_runtime_snapshot.nearest_match.as_deref(),
            Some("current nearest"),
            "a worker result that arrives while disabled must not publish stale nearest-match text"
        );
        assert_eq!(
            owner.media_match_runtime_snapshot.last_evidence.as_deref(),
            Some("current evidence"),
            "a worker result that arrives while disabled must not publish stale evidence"
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("canceled: checkpoint kept")
        );
    }

    #[test]
    fn background_warmup_waits_for_unresolved_room_media_before_full_root_scan() {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_background_warmup_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.active_shared_playlist_index = Some(0);

        owner.maybe_queue_media_match_background_warmup(&handle, &mut state);

        assert!(
            owner.media_match_background_worker_rx.is_none(),
            "automatic warmup should not launch a full-root scan while the room target is unresolved"
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("idle: waiting for resolved local media")
        );
    }

    #[test]
    fn background_warmup_cancels_running_full_root_scan_for_unresolved_room_media() {
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_background_warmup_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.active_shared_playlist_index = Some(0);
        let (_tx, rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(Arc::clone(&cancel_flag));

        owner.maybe_queue_media_match_background_warmup(&handle, &mut state);

        assert!(
            cancel_flag.load(Ordering::Relaxed),
            "automatic warmup should cancel a stale full-root scan once unresolved room media appears"
        );
        assert_eq!(
            owner.media_match_background_cancel_disposition,
            Some(GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint)
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("canceling background warmup: waiting for resolved local media")
        );
    }

    #[test]
    fn exact_shared_playlist_match_skips_background_fingerprinting_without_wire_sharing() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-exact-skip-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(false),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode.mkv")
                .with_path(media_path.to_string_lossy().into_owned()),
        );
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert_eq!(
            owner.media_match_exact_playlist_plan_for_state(&state, &root),
            GuiMediaMatchExactPlaylistPlan::ExactNoFingerprint {
                path: media_path.to_string_lossy().into_owned(),
            }
        );
        assert!(owner.queue_media_match_background_worker(
            &handle,
            &mut state,
            "test exact",
            false,
            true,
        ));
        assert!(owner.media_match_background_worker_rx.is_none());
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("idle: exact shared-playlist file already loaded")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_exact_shared_playlist_match_needs_only_signature_fingerprint_when_sharing_enabled() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-exact-signature-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode.mkv")
                .with_path(media_path.to_string_lossy().into_owned()),
        );
        owner.remember_local_shared_playlist_media_match_signature_path(
            &media_path.to_string_lossy(),
        );

        assert_eq!(
            owner.media_match_exact_playlist_plan_for_state(&state, &root),
            GuiMediaMatchExactPlaylistPlan::ExactNeedsSignature {
                path: media_path.to_string_lossy().into_owned(),
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_exact_shared_playlist_match_does_not_need_signature_fingerprint_when_sharing_enabled()
    {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-remote-exact-no-signature-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode.mkv")
                .with_path(media_path.to_string_lossy().into_owned()),
        );
        let (_tx, rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(cancel_flag.clone());
        owner.media_match_background_trigger_key = Some("background warmup".to_owned());
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        assert_eq!(
            owner.media_match_exact_playlist_plan_for_state(&state, &root),
            GuiMediaMatchExactPlaylistPlan::ExactNoFingerprint {
                path: media_path.to_string_lossy().into_owned(),
            }
        );
        owner.maybe_queue_media_match_exact_playlist_signature(&handle, &mut state);

        assert!(
            !cancel_flag.load(Ordering::Relaxed),
            "a remote exact shared-playlist receiver must not preempt broad work for signature sharing"
        );
        assert_eq!(owner.media_match_background_cancel_disposition, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exact_playlist_signature_sharing_runs_when_background_warmup_is_disabled() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-exact-no-warmup-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_path = root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            media_match_background_warmup_enabled: Some(false),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode.mkv")
                .with_path(media_path.to_string_lossy().into_owned()),
        );
        owner.remember_local_shared_playlist_media_match_signature_path(
            &media_path.to_string_lossy(),
        );
        let (_tx, rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        owner.media_match_background_worker_rx = Some(rx);
        owner.media_match_background_worker_cancel = Some(cancel_flag.clone());
        owner.media_match_background_trigger_key = Some("background warmup".to_owned());
        let handle = GuiQueuedRuntimeBridgeHandle::default();

        owner.maybe_queue_media_match_exact_playlist_signature(&handle, &mut state);

        assert!(
            cancel_flag.load(Ordering::Relaxed),
            "exact playlist signature sharing must preempt broad warmup work even when warmup is disabled"
        );
        assert_eq!(
            owner
                .media_match_runtime_snapshot
                .background_status
                .as_deref(),
            Some("canceling broad Media Matching work for exact playlist fingerprint")
        );
        assert_eq!(
            owner.media_match_background_cancel_disposition,
            Some(GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exact_shared_playlist_plan_does_not_treat_different_path_bearing_target_as_exact() {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-gui-media-match-runtime-exact-path-context-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("test root should be created");
        let config_path = root.join("sorotte.ini");
        let media_path = root.join("episode.mkv");
        let other_root = root.join("other");
        std::fs::create_dir_all(&other_root).expect("other root should be created");
        let other_path = other_root.join("episode.mkv");
        std::fs::write(&media_path, b"not real media").expect("media file should be created");
        let saved_settings = StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
        state.apply_shared_playlist_entries(
            vec![other_path.to_string_lossy().into_owned()],
            Some(0),
            false,
        );
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        owner.active_shared_playlist_index = Some(0);
        owner.player_local_file = Some(
            sorotte_player_api::LocalFileUpdate::new("episode.mkv")
                .with_path(media_path.to_string_lossy().into_owned()),
        );

        assert_eq!(
            owner.media_match_exact_playlist_plan_for_state(&state, &root),
            GuiMediaMatchExactPlaylistPlan::None
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
