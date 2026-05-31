use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use sorotte_media_match::{
    MediaExtractionSettings, MediaMatchDecision, MediaMatchTier,
    decide_media_match_against_wire_signature, media_match_wire_signature_from_value,
};

use crate::app::media_match_support::{
    discard_media_match_index_rebuild_backup, media_match_record_for_path,
    media_match_sqlite_index_exists, media_match_tier_label,
    prepare_media_match_index_rebuild_backup, restore_media_match_index_rebuild_backup,
};

use super::super::{
    GuiMediaMatchBackgroundCancelDisposition, GuiMediaMatchBackgroundWorkerEvent,
    GuiMediaMatchIndexRebuildBackup, GuiMediaMatchToolWorkerEvent,
};
use super::*;

#[derive(Debug, Clone)]
struct GuiMediaMatchRemoteTarget {
    target_file_name: String,
    media_match_signature: serde_json::Value,
}

fn media_match_full_verify_extraction_settings() -> MediaExtractionSettings {
    MediaExtractionSettings::audio_constellation_v3()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuiMediaMatchExactPlaylistPlan {
    None,
    ExactNoFingerprint { path: String },
    ExactNeedsSignature { path: String },
}

impl GuiPersistedConfigRuntimeOwner {
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
        let mut tiers = BTreeMap::new();
        let status = if !projected_state.media_match.settings.fingerprinting_enabled {
            "disabled: fingerprinting off".to_owned()
        } else if !projected_state.media_match.settings.wire_sharing_enabled {
            "disabled: sharing off".to_owned()
        } else if self.media_match_runtime_snapshot.health != GuiMediaMatchToolHealth::Healthy {
            "unavailable: tools unhealthy".to_owned()
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
            let current_path = if let Some(path) =
                self.media_match_current_local_path_for_state(projected_state)
            {
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
                &media_match_full_verify_extraction_settings(),
            ) else {
                let status = "pending local fingerprint".to_owned();
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
                    let Some(value) = peer_state.media_match_signature else {
                        summaries.push(format!("{username}: unavailable"));
                        continue;
                    };
                    match media_match_wire_signature_from_value(&value) {
                        Ok(signature) => {
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
                        Err(_) => summaries.push(format!("{username}: incompatible")),
                    }
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
            .media_match_current_local_path_for_state(projected_state)
            .unwrap_or_default();
        let remote_peer_states = self
            .session
            .as_ref()
            .map(|session| session.current_room_media_match_peer_file_states())
            .unwrap_or_default();
        let remote_signature_token = format!("{remote_peer_states:?}");
        format!(
            "{}|{}|{}|{:?}|{:?}|{}",
            current_path,
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
        if self.media_match_runtime_snapshot.remote_status.as_deref() == Some(status.as_str()) {
            return;
        }
        let mut snapshot = self.media_match_runtime_snapshot.clone();
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

    fn request_media_match_background_worker_cancel(
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
            && media_match_record_for_path(
                root,
                &path,
                &media_match_full_verify_extraction_settings(),
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
                let extraction_settings = media_match_full_verify_extraction_settings();
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
        let playlist_target = self.current_shared_playlist_target(projected_state);
        self.session
            .as_ref()
            .map(|session| session.current_room_media_match_peer_file_states())
            .unwrap_or_default()
            .into_iter()
            .filter(|peer| peer.has_file)
            .filter_map(|peer| {
                let media_match_signature = peer.media_match_signature?;
                let target_file_name = Self::usable_media_match_peer_file_name(peer.file_name)
                    .or_else(|| playlist_target.clone())?;
                Some(GuiMediaMatchRemoteTarget {
                    target_file_name,
                    media_match_signature,
                })
            })
            .collect()
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

    pub(in crate::app::runtime_owner) fn media_match_cached_room_candidate_for_target(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        target: &str,
    ) -> Option<String> {
        if !projected_state.media_match.settings.fingerprinting_enabled {
            return None;
        }
        let root = self.media_match_root_for_request(projected_state)?;
        let search_roots = self.automatic_media_search_roots(projected_state);
        if search_roots.is_empty() {
            return None;
        }
        let room_target = Path::new(target)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(target);
        let remote_targets = self.media_match_remote_targets_for_state(projected_state);
        let matching_targets = remote_targets.iter().filter(|remote| {
            remote.target_file_name.eq_ignore_ascii_case(target)
                || Path::new(&remote.target_file_name)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(room_target))
        });
        for remote in matching_targets {
            if let Some(candidate) = media_match_cached_strong_candidate_for_remote_signature(
                &root,
                &search_roots,
                &remote.target_file_name,
                &remote.media_match_signature,
                &projected_state.media_match.settings,
                &media_match_full_verify_extraction_settings(),
            ) {
                return Some(candidate.path);
            }
        }
        None
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
        if let Some(path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.clone())
            .filter(|path| Path::new(path).is_file())
        {
            return Some(path);
        }

        let target = self.media_match_room_target_for_state(projected_state)?;
        match self.resolve_main_window_user_media_target(projected_state, &target) {
            Ok(GuiUserMediaTargetResolution::Resolved(path)) if Path::new(&path).is_file() => {
                Some(path)
            }
            Ok(GuiUserMediaTargetResolution::Resolved(_))
            | Ok(GuiUserMediaTargetResolution::Pending | GuiUserMediaTargetResolution::Missing)
            | Err(_) => None,
        }
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
                    if !candidate.is_file() {
                        continue;
                    }
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
                            rebuild_persisted_media_match_remote_candidates_with_progress_and_cancel(
                                MediaMatchRemoteCandidateRebuildRequest {
                                    root: &root,
                                    search_roots: &search_roots,
                                    candidates: None,
                                    target_file_name: &remote_candidate.target_file_name,
                                    media_match_signature: &remote_candidate.media_match_signature,
                                    settings: &settings,
                                    tools: &tools,
                                    extraction_settings: &extraction_settings,
                                    cancel_flag: Some(worker_cancel_flag.as_ref()),
                                },
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
                let strong_fast_match = fast_result
                    .as_ref()
                    .ok()
                    .and_then(|result| result.current_decision.as_deref())
                    .is_some_and(|decision| {
                        decision.starts_with("strong:") || decision.starts_with("probable:")
                    });
                let remote_promotion_candidates = (current_player_path.is_none())
                    .then(|| {
                        fast_result
                            .as_ref()
                            .ok()
                            .map(|result| result.full_promotion_candidates.clone())
                            .filter(|candidates| !candidates.is_empty())
                    })
                    .flatten();
                if !strong_fast_match && remote_promotion_candidates.is_none() {
                    let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(fast_result));
                    return;
                }
                let Some(mut hardening_candidates) = current_player_path
                    .is_some()
                    .then(|| candidates.clone())
                    .flatten()
                    .or(remote_promotion_candidates)
                else {
                    let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(fast_result));
                    return;
                };
                let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::FastResult(
                    fast_result.clone(),
                ));
                if worker_cancel_flag.load(Ordering::Relaxed) {
                    let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(Err(
                        "Media Matching index rebuild was canceled.".to_owned(),
                    )));
                    return;
                }
                let sampled_extraction_settings =
                    sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
                if current_player_path.is_some() {
                    hardening_candidates = media_match_full_promotion_candidates_for_current(
                        &root,
                        &hardening_candidates,
                        current_player_path.as_deref(),
                        &settings,
                        &sampled_extraction_settings,
                        MEDIA_MATCH_MAX_FULL_PROMOTIONS_PER_QUERY,
                    );
                } else {
                    hardening_candidates.truncate(MEDIA_MATCH_MAX_FULL_PROMOTIONS_PER_QUERY.max(1));
                }
                let extraction_settings =
                    media_match_full_verify_extraction_settings();
                let full_result =
                    media_match_tool_paths_for_settings(&root, &extraction_settings).and_then(
                        |tools| {
                            if let Some(remote_candidate) = remote_candidate {
                                rebuild_persisted_media_match_remote_candidates_with_progress_and_cancel(
                                    MediaMatchRemoteCandidateRebuildRequest {
                                        root: &root,
                                        search_roots: &search_roots,
                                        candidates: Some(hardening_candidates),
                                        target_file_name: &remote_candidate.target_file_name,
                                        media_match_signature: &remote_candidate.media_match_signature,
                                        settings: &settings,
                                        tools: &tools,
                                        extraction_settings: &extraction_settings,
                                        cancel_flag: Some(worker_cancel_flag.as_ref()),
                                    },
                                    |progress| {
                                        let _ = progress_tx.send(
                                            GuiMediaMatchBackgroundWorkerEvent::Progress(progress),
                                        );
                                    },
                                )
                            } else {
                                rebuild_persisted_media_match_candidates_with_progress_and_cancel(
                                    MediaMatchCandidateRebuildRequest {
                                        root: &root,
                                        candidates: hardening_candidates,
                                        current_player_path: current_player_path.as_deref(),
                                        settings: &settings,
                                        tools: &tools,
                                        extraction_settings: &extraction_settings,
                                        cancel_flag: Some(worker_cancel_flag.as_ref()),
                                    },
                                    |progress| {
                                        let _ = progress_tx.send(
                                            GuiMediaMatchBackgroundWorkerEvent::Progress(progress),
                                        );
                                    },
                                )
                            }
                        },
                    );
                let full_result = full_result.map(|mut result| {
                    result.message =
                        format!("Media Matching full hardening complete. {}", result.message);
                    result
                });
                let _ = tx.send(GuiMediaMatchBackgroundWorkerEvent::Finished(full_result));
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
        self.media_match_wire_sync_token = None;
        self.last_attached_media_resolution_trigger = None;
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
        loop {
            match rx.try_recv() {
                Ok(GuiMediaMatchBackgroundWorkerEvent::Progress(progress)) => {
                    self.publish_media_match_background_status(
                        handle,
                        projected_state,
                        Self::media_match_background_progress_status(&progress),
                    );
                }
                Ok(GuiMediaMatchBackgroundWorkerEvent::FastResult(result)) => match result {
                    Ok(result) => {
                        if !self.apply_media_match_background_result(
                            handle,
                            projected_state,
                            result,
                            false,
                            "full hardening queued",
                        ) {
                            break;
                        }
                    }
                    Err(error) => {
                        let mut snapshot = self.refresh_media_match_runtime_snapshot(
                            &projected_state.media_match.settings,
                        );
                        snapshot.message = Some(error.clone());
                        snapshot.background_status = Some("failed".to_owned());
                        self.media_match_runtime_snapshot = snapshot.clone();
                        Self::push_actions_and_project(
                            handle,
                            projected_state,
                            vec![GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(snapshot)],
                        );
                    }
                },
                Ok(GuiMediaMatchBackgroundWorkerEvent::Finished(result)) => {
                    keep_rx = false;
                    self.media_match_background_worker_cancel = None;
                    if !matches!(&result, Err(error) if error.contains("canceled")) {
                        self.media_match_background_cancel_disposition = None;
                    }
                    match result {
                        Ok(result) => {
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
                            let disposition = self
                                .media_match_background_cancel_disposition
                                .take()
                                .unwrap_or(
                                    GuiMediaMatchBackgroundCancelDisposition::RestorePrevious,
                                );
                            let restore_result = self.finish_media_match_background_index_backup(
                                disposition
                                    == GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                            );
                            let status = match (disposition, restore_result) {
                                (
                                    GuiMediaMatchBackgroundCancelDisposition::RestorePrevious,
                                    Ok(()),
                                ) => "canceled: previous index restored".to_owned(),
                                (
                                    GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint,
                                    Ok(()),
                                ) => "canceled: checkpoint kept".to_owned(),
                                (_, Err(error)) => {
                                    format!("canceled: restore failed: {error}")
                                }
                            };
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
                        }
                        Err(error) => {
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
                    keep_rx = false;
                    self.media_match_background_worker_cancel = None;
                    self.media_match_background_cancel_disposition = None;
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
        if keep_rx {
            self.media_match_background_worker_rx = Some(rx);
        }
    }

    pub(in crate::app::runtime_owner) fn maybe_queue_media_match_background_warmup(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
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
        if self.media_match_background_worker_rx.is_some() {
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

    pub(super) fn handle_install_media_match_tools_request(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) -> bool {
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
                    message: "Canceling Media Matching background work before clearing cache. Run Clear Cache again when it is idle.".to_owned(),
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
    use crate::app::runtime_owner::{GuiAttachedMediaSearchIndex, GuiAttachedMediaSearchRootIndex};
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
                GuiMediaMatchBackgroundWorkerEvent::FastResult(_) => {}
                GuiMediaMatchBackgroundWorkerEvent::Finished(result) => break result,
            }
        }
        .expect("inventory-only worker should succeed without media tools");
        owner
            .finish_media_match_background_index_backup(true)
            .expect("backup should be discarded");

        let connection = rusqlite::Connection::open(
            root.join("cache")
                .join("media-match")
                .join("index-v3.sqlite3"),
        )
        .expect("SQLite index should open");
        let inventory = connection
            .query_row("SELECT COUNT(*) FROM media_files_v3", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("inventory count should load");
        let fingerprints = connection
            .query_row("SELECT COUNT(*) FROM fingerprints_v3", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("fingerprint count should load");

        assert_eq!(inventory, 1);
        assert_eq!(fingerprints, 0);
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
    fn exact_shared_playlist_match_needs_only_signature_fingerprint_when_sharing_enabled() {
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

        assert_eq!(
            owner.media_match_exact_playlist_plan_for_state(&state, &root),
            GuiMediaMatchExactPlaylistPlan::ExactNeedsSignature {
                path: media_path.to_string_lossy().into_owned(),
            }
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
