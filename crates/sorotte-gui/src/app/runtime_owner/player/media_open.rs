use super::*;
use crate::app::runtime_stack::GuiPlaylistProtocolDeliveryFence;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn open_main_window_user_media_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved { path, .. }) => path,
            Ok(GuiUserMediaTargetResolution::Ambiguous { candidate_count }) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!(
                        "Could not choose local user media because {candidate_count} equally credible files matched; use a more specific path."
                    ),
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!("Indexing media library to resolve user media: {target}."),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if projected_state.playlist_backed_media_opens_preferred() {
            self.open_media_files_through_shared_playlist_runtime_impl(
                handle,
                projected_state,
                vec![resolved_target],
                None,
            );
            return;
        }

        self.ensure_configured_player_attached();
        if self.player.is_some() {
            if browser_stream_target_kind(&resolved_target, None)
                == GuiStreamTargetKind::ExtractorPageUrl
                && !projected_state
                    .plugin_enablement
                    .enabled_for(GuiPluginSelection::StreamSupport)
            {
                Self::push_runtime_unavailable(
                    handle,
                    "Stream Support is disabled; extractor-backed URLs cannot be opened until it is enabled.".to_owned(),
                );
                return;
            }
            if !self.preflight_user_stream_target(&resolved_target) {
                return;
            }
            self.prepare_stream_load_tracking(&resolved_target, true);
            self.open_media_files_through_attached_player_impl(handle, vec![resolved_target]);
        } else {
            Self::push_runtime_unavailable(
                handle,
                self.open_media_unavailable_message_impl(&[resolved_target]),
            );
        }
    }

    fn project_loaded_shared_playlist_into_state(
        projected_state: &mut SorotteGuiShellAppState,
        entries: Vec<String>,
        selected_index: Option<usize>,
        fresh_row_identities: bool,
    ) -> bool {
        let entries = SorotteGuiShellAppState::normalize_shared_playlist_entries(entries);
        let selected_index = selected_index
            .filter(|_| !entries.is_empty())
            .map(|index| index.min(entries.len().saturating_sub(1)));
        projected_state.main_window.shared_playlist_enabled = true;
        if fresh_row_identities {
            projected_state.remember_shared_playlist_undo_snapshot();
        } else {
            projected_state.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        }
        let fresh_source_states = fresh_row_identities.then(|| {
            entries
                .iter()
                .map(|entry| projected_state.playlist_source_state_for_entry(entry))
                .collect::<Vec<_>>()
        });
        projected_state.apply_shared_playlist_entries(entries.clone(), selected_index, false);
        if let Some(fresh_source_states) = fresh_source_states {
            for (row, mut source_state) in projected_state
                .main_window
                .playlist
                .iter_mut()
                .zip(fresh_source_states)
            {
                let entry_id = GuiPlaylistEntryId::next();
                source_state.entry_id = entry_id;
                row.entry_id = entry_id;
                row.source_state = source_state;
            }
        }
        projected_state.main_window.active_playlist_index = selected_index;
        true
    }

    fn mark_shared_playlist_entry_as_local_source(
        projected_state: &mut SorotteGuiShellAppState,
        index: usize,
    ) -> bool {
        let Some(mut source_state) = projected_state
            .main_window
            .playlist
            .get(index)
            .map(|row| row.source_state.clone())
        else {
            return false;
        };
        if matches!(
            source_state.policy,
            GuiPlaylistSourcePolicy::ForceMediaMatching | GuiPlaylistSourcePolicy::ForcePlex
        ) {
            return false;
        }
        source_state.set_resolved_provider(GuiMediaSourceProviderId::local());
        source_state.status = GuiPlaylistSourceStatus::Available;
        source_state.detail = Some("Added from the local filesystem.".to_owned());
        source_state.resolution_steps.clear();
        projected_state.set_playlist_source_state(index, source_state)
    }

    fn mark_bound_shared_playlist_entries_as_local_sources(
        projected_state: &mut SorotteGuiShellAppState,
        bound_row_ids: &[GuiPlaylistEntryId],
    ) -> bool {
        let matching_indices = projected_state
            .main_window
            .playlist
            .iter()
            .enumerate()
            .filter_map(|(index, row)| bound_row_ids.contains(&row.entry_id).then_some(index))
            .collect::<Vec<_>>();
        let mut changed = false;
        for index in matching_indices {
            changed |= Self::mark_shared_playlist_entry_as_local_source(projected_state, index);
        }
        changed
    }

    fn mark_unavailable_shared_playlist_local_origins(
        projected_state: &mut SorotteGuiShellAppState,
        unavailable_row_ids: &[GuiPlaylistEntryId],
    ) -> bool {
        let mut changed = false;
        for row in &mut projected_state.main_window.playlist {
            if !unavailable_row_ids.contains(&row.entry_id)
                || !matches!(
                    row.source_state.policy,
                    GuiPlaylistSourcePolicy::Automatic | GuiPlaylistSourcePolicy::ForceLocal
                )
                || row.source_state.current_provider_id != GuiMediaSourceProviderId::local()
            {
                continue;
            }
            let source_state = &mut row.source_state;
            let next_detail = Some("The selected local file is no longer available.".to_owned());
            if source_state.status != GuiPlaylistSourceStatus::Missing
                || source_state.detail != next_detail
            {
                source_state.status = GuiPlaylistSourceStatus::Missing;
                source_state.detail = next_detail;
                source_state.resolution_steps.clear();
                changed = true;
            }
        }
        changed
    }

    fn shared_playlist_rows_corresponding_to_dispatch(
        state: &SorotteGuiShellAppState,
        dispatch: &GuiSharedPlaylistOpenDispatch,
        previous_row_count: usize,
        opened_entry_count: usize,
        playlist_insert_slot: Option<usize>,
        selected_playlist_index: Option<usize>,
    ) -> Vec<(GuiPlaylistEntryId, String)> {
        if playlist_insert_slot.is_none() {
            return state
                .main_window
                .playlist
                .iter()
                .take(dispatch.items.len())
                .map(|row| (row.entry_id, row.label.clone()))
                .collect();
        }

        let inserted_start = playlist_insert_slot
            .unwrap_or_default()
            .min(previous_row_count);
        let inserted_end = inserted_start
            .saturating_add(opened_entry_count)
            .min(state.main_window.playlist.len());
        let mut used_indices = BTreeSet::new();
        let mut rows = Vec::new();
        for item in &dispatch.items {
            let selected_match = selected_playlist_index.filter(|index| {
                !used_indices.contains(index)
                    && state
                        .main_window
                        .playlist
                        .get(*index)
                        .is_some_and(|row| row.label == item.published_entry)
            });
            let inserted_match = (inserted_start..inserted_end).find(|index| {
                !used_indices.contains(index)
                    && state
                        .main_window
                        .playlist
                        .get(*index)
                        .is_some_and(|row| row.label == item.published_entry)
            });
            let matching_index = selected_match.or(inserted_match).or_else(|| {
                state
                    .main_window
                    .playlist
                    .iter()
                    .enumerate()
                    .position(|(index, row)| {
                        !used_indices.contains(&index) && row.label == item.published_entry
                    })
            });
            let Some(index) = matching_index else {
                continue;
            };
            used_indices.insert(index);
            if let Some(row) = state.main_window.playlist.get(index) {
                rows.push((row.entry_id, row.label.clone()));
            }
        }
        rows
    }

    fn selected_playlist_local_origin(
        &mut self,
        state: &SorotteGuiShellAppState,
        selected_playlist_index: Option<usize>,
    ) -> Option<String> {
        let entry_id = state
            .main_window
            .playlist
            .get(selected_playlist_index?)?
            .entry_id;
        self.local_shared_playlist_media_path_for_row(state, entry_id)
    }

    fn open_selected_playlist_media_after_shared_playlist_projection(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        selected_playlist_index: Option<usize>,
        selected_media_source_path: Option<String>,
    ) -> SelectedPlaylistMediaSyncOutcome {
        let Some(selected_index) = selected_playlist_index else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        let Some((row_id, row_label, source_policy, preferred_provider)) = projected_state
            .main_window
            .playlist
            .get(selected_index)
            .map(|row| {
                (
                    row.entry_id,
                    row.label.clone(),
                    row.source_state.policy,
                    row.source_state
                        .preferred_provider_id()
                        .cloned()
                        .unwrap_or_else(|| row.source_state.current_provider_id.clone()),
                )
            })
        else {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        };
        if matches!(
            source_policy,
            GuiPlaylistSourcePolicy::Automatic
                | GuiPlaylistSourcePolicy::ForceLocal
                | GuiPlaylistSourcePolicy::PreferMediaMatching
        ) {
            let selected_path = selected_media_source_path.filter(|path| Path::new(path).is_file());
            let Some(selected_path) = selected_path else {
                self.playlist_resolution
                    .local_origins_by_row
                    .remove(&row_id);
                self.last_attached_media_resolution_trigger = None;
                return self
                    .sync_selected_shared_playlist_media_to_attached_player_impl(projected_state);
            };
            self.clear_plex_stream_resolution_state();
            self.ensure_playlist_resolution_attempt(
                row_id,
                self.playlist_resolution.generation,
                &row_label,
                source_policy,
            );
            return self.open_selected_playlist_media_path_through_attached_player_impl(
                projected_state,
                &[selected_path],
            );
        }

        let target = row_label;
        let provider_id = preferred_provider;
        if !self.preflight_room_stream_target(projected_state, &target) {
            return SelectedPlaylistMediaSyncOutcome::NoChange;
        }
        self.sync_selected_playlist_source_override_to_attached_player(
            projected_state,
            &target,
            &provider_id,
        )
    }

    fn open_system_folder(path: &Path, description: &str) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg(path);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(path);
            command
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(path);
            command
        };

        command.spawn().map_err(|error| {
            format!(
                "Opening {description} at '{}' failed: {error}",
                path.display(),
            )
        })?;
        Ok(())
    }

    fn open_system_file_browser_for_path(path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err(format!(
                "Could not open a containing folder for '{}': no parent directory exists.",
                path.display()
            ));
        };
        Self::open_system_folder(parent, "the containing folder")
    }

    pub(in crate::app::runtime_owner) fn open_main_window_user_containing_folder_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target = match self
            .resolve_main_window_user_media_target(projected_state, &target)
        {
            Ok(GuiUserMediaTargetResolution::Resolved { path, .. }) => path,
            Ok(GuiUserMediaTargetResolution::Ambiguous { candidate_count }) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!(
                        "Could not choose a containing folder because {candidate_count} equally credible local files matched; use a more specific path."
                    ),
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Pending) => {
                Self::push_actions_and_project(
                    handle,
                    projected_state,
                    vec![GuiShellAction::PushTransientNotification {
                        level: GuiTransientNotificationLevel::Info,
                        message: format!(
                            "Indexing media library to resolve a local path for user media: {target}."
                        ),
                    }],
                );
                return;
            }
            Ok(GuiUserMediaTargetResolution::Missing) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Could not find a local path for user media: {target}."),
                );
                return;
            }
            Err(error) => {
                Self::push_runtime_error_notification(
                    handle,
                    projected_state,
                    format!("Resolving user media '{target}' failed: {error}"),
                );
                return;
            }
        };

        if browser_is_url(&resolved_target) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!("Cannot open a containing folder for the stream URL: {resolved_target}."),
            );
            return;
        }

        if let Err(error) = Self::open_system_file_browser_for_path(Path::new(&resolved_target)) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }

    pub(in crate::app::runtime_owner) fn open_media_files_through_shared_playlist_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        paths: Vec<String>,
        playlist_insert_slot: Option<usize>,
    ) {
        let selected_paths = paths
            .into_iter()
            .filter_map(|path| normalized_editable_text(&path))
            .collect::<Vec<_>>();
        if selected_paths.is_empty() {
            return;
        }

        let dispatch = match self.shared_playlist_open_dispatch_for_selected_paths_impl(
            projected_state,
            selected_paths.clone(),
        ) {
            Ok(dispatch) => dispatch,
            Err(error) => {
                Self::push_runtime_unavailable(handle, error);
                return;
            }
        };
        self.open_shared_playlist_dispatch_runtime_impl(
            handle,
            projected_state,
            selected_paths,
            dispatch,
            playlist_insert_slot,
        );
    }

    pub(in crate::app::runtime_owner) fn open_shared_playlist_dispatch_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        selected_paths: Vec<String>,
        dispatch: GuiSharedPlaylistOpenDispatch,
        playlist_insert_slot: Option<usize>,
    ) {
        self.open_shared_playlist_dispatch_after_prior_delivery_fence(
            handle,
            projected_state,
            selected_paths,
            dispatch,
            playlist_insert_slot,
        );
    }

    fn open_shared_playlist_dispatch_after_prior_delivery_fence(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        selected_paths: Vec<String>,
        dispatch: GuiSharedPlaylistOpenDispatch,
        playlist_insert_slot: Option<usize>,
    ) {
        if self.pending_shared_playlist_open.is_some() {
            Self::push_runtime_unavailable(
                handle,
                "Another shared-playlist media open is waiting for its session update to be delivered. Try again after it completes."
                    .to_owned(),
            );
            return;
        }

        let current_playlist_entry_count = projected_state.main_window.playlist.len();
        let current_playlist_index =
            self.shared_playlist_mutation_current_index(projected_state, false);
        let (playlist_entries, selected_playlist_index) = projected_state
            .shared_playlist_entries_after_media_open_from_state_with_current_index(
                dispatch.playlist_entries(),
                playlist_insert_slot,
                current_playlist_index,
            );
        let opened_entry_count = if playlist_insert_slot.is_some() {
            playlist_entries
                .len()
                .saturating_sub(current_playlist_entry_count)
        } else {
            playlist_entries.len()
        };
        if playlist_insert_slot.is_some() && opened_entry_count == 0 {
            let rows = Self::shared_playlist_rows_corresponding_to_dispatch(
                projected_state,
                &dispatch,
                current_playlist_entry_count,
                opened_entry_count,
                playlist_insert_slot,
                selected_playlist_index,
            );
            let binding_outcome =
                self.remember_local_shared_playlist_media_paths(projected_state, &dispatch, &rows);
            let source_changed = Self::mark_bound_shared_playlist_entries_as_local_sources(
                projected_state,
                &binding_outcome.bound_row_ids,
            ) | Self::mark_unavailable_shared_playlist_local_origins(
                projected_state,
                &binding_outcome.unavailable_row_ids,
            );
            if !binding_outcome.bound_row_ids.is_empty()
                || !binding_outcome.unavailable_row_ids.is_empty()
            {
                self.last_attached_media_resolution_trigger = None;
                if source_changed {
                    handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                        MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                    ));
                }
                let selected_media_sync = self
                    .sync_selected_shared_playlist_media_to_attached_player_impl(projected_state);
                self.sync_session_playstate_to_attached_player_impl(
                    projected_state,
                    selected_media_sync.selection_handoff_ready(
                        self.session.as_ref().is_some_and(|session| {
                            session.has_pending_playlist_index_reset_intent()
                        }),
                    ),
                );
            }
            return;
        }

        if self.session.is_none() {
            self.ensure_configured_player_attached();
            if self.player.is_none() {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }

            if playlist_insert_slot.is_none() {
                self.begin_shared_playlist_replacement_scope();
            }
            if !Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
                playlist_insert_slot.is_none(),
            ) {
                Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                );
                return;
            }
            let opened_rows = Self::shared_playlist_rows_corresponding_to_dispatch(
                projected_state,
                &dispatch,
                current_playlist_entry_count,
                opened_entry_count,
                playlist_insert_slot,
                selected_playlist_index,
            );
            let binding_outcome = self.remember_local_shared_playlist_media_paths(
                projected_state,
                &dispatch,
                &opened_rows,
            );
            Self::mark_bound_shared_playlist_entries_as_local_sources(
                projected_state,
                &binding_outcome.bound_row_ids,
            );
            Self::mark_unavailable_shared_playlist_local_origins(
                projected_state,
                &binding_outcome.unavailable_row_ids,
            );
            let selected_media_source_path =
                self.selected_playlist_local_origin(projected_state, selected_playlist_index);
            self.active_shared_playlist_index = selected_playlist_index;
            Self::push_actions_and_project(
                handle,
                projected_state,
                vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                    MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
                )],
            );

            let selected_media_sync = self
                .open_selected_playlist_media_after_shared_playlist_projection(
                    projected_state,
                    selected_playlist_index,
                    selected_media_source_path,
                );
            let selection_handoff_ready = selected_media_sync.selection_handoff_ready(false);
            self.sync_session_playstate_to_attached_player_impl(
                projected_state,
                selection_handoff_ready,
            );

            let success_message =
                Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            let warning = self.shared_playlist_session_unavailable_message_impl();
            handle.push_actions([
                GuiShellAction::SwitchView(GuiShellView::Room),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: success_message.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(success_message),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: warning.clone(),
                },
                GuiShellAction::AnnounceSystemChatEvent(warning),
            ]);
            return;
        }

        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.playlist_control_available())
        {
            Self::push_runtime_unavailable(
                handle,
                self.shared_playlist_control_unavailable_message_impl(),
            );
            return;
        }

        let Some(session) = self.session.as_mut() else {
            Self::push_runtime_unavailable(
                handle,
                self.shared_playlist_session_unavailable_message_impl(),
            );
            return;
        };
        let session_result = session.replace_playlist_with_delivery_fence(
            playlist_entries.clone(),
            selected_playlist_index,
        );
        let session_success = session_result.is_ok();
        if session_success {
            self.active_shared_playlist_index = selected_playlist_index;
        }
        if session_success && playlist_insert_slot.is_none() {
            self.begin_shared_playlist_replacement_scope();
        }
        let session_playlist_projected = session_success
            && Self::project_loaded_shared_playlist_into_state(
                projected_state,
                playlist_entries.clone(),
                selected_playlist_index,
                playlist_insert_slot.is_none(),
            );
        let selected_media_source_path = if session_playlist_projected {
            let opened_rows = Self::shared_playlist_rows_corresponding_to_dispatch(
                projected_state,
                &dispatch,
                current_playlist_entry_count,
                opened_entry_count,
                playlist_insert_slot,
                selected_playlist_index,
            );
            let binding_outcome = self.remember_local_shared_playlist_media_paths(
                projected_state,
                &dispatch,
                &opened_rows,
            );
            Self::mark_bound_shared_playlist_entries_as_local_sources(
                projected_state,
                &binding_outcome.bound_row_ids,
            );
            Self::mark_unavailable_shared_playlist_local_origins(
                projected_state,
                &binding_outcome.unavailable_row_ids,
            );
            let selected_media_source_path =
                self.selected_playlist_local_origin(projected_state, selected_playlist_index);
            if let Some(path) = selected_media_source_path.as_ref() {
                self.remember_local_shared_playlist_media_match_signature_path(path);
            }
            handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                MainWindowRuntimeSnapshot::from_shell_state(&projected_state.main_window),
            ));
            let delivery_fence = if self.session_transport.is_some() {
                session_result
                    .as_ref()
                    .expect("successful playlist mutation should retain its delivery fence")
                    .clone()
            } else {
                // In-process adapter tests and embedders can own a session
                // without installing a transport. That seam is synchronous:
                // no transport means there can be no later write receipt to
                // satisfy, while production loopback/TCP sessions always
                // install a handle and retain the exact causal frontier.
                GuiPlaylistProtocolDeliveryFence::default()
            };
            if !delivery_fence.is_reached() {
                // Retain the continuation before pumping. Immediate and
                // threaded transports then advance the same exact frame
                // frontier through their terminal write receipts. Unrelated
                // coalescible frames can disappear without stranding it.
                self.pending_shared_playlist_open =
                    Some(GuiPendingSharedPlaylistOpen::AfterMutation {
                        dispatch,
                        opened_entry_count,
                        selected_playlist_index,
                        selected_media_source_path,
                        delivery_fence,
                    });
                match self.drain_session_transport_outbound_before_synchronous_player_open(
                    handle,
                    projected_state,
                ) {
                    GuiSessionOutboundDrainDisposition::Drained
                    | GuiSessionOutboundDrainDisposition::Pending => {
                        self.resume_pending_shared_playlist_open_if_ready(handle, projected_state);
                    }
                    GuiSessionOutboundDrainDisposition::Failed => {
                        self.pending_shared_playlist_open = None;
                    }
                }
                return;
            }
            selected_media_source_path
        } else {
            None
        };
        self.finish_shared_playlist_open_after_delivery(
            handle,
            projected_state,
            GuiSharedPlaylistOpenCompletion {
                dispatch,
                opened_entry_count,
                selected_playlist_index,
                selected_media_source_path,
                session_playlist_projected,
                session_success,
                session_error: session_result.err(),
            },
        );
    }

    fn finish_shared_playlist_open_after_delivery(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        completion: GuiSharedPlaylistOpenCompletion,
    ) {
        let dispatch = completion.dispatch;
        let opened_entry_count = completion.opened_entry_count;
        let selected_playlist_index = completion.selected_playlist_index;
        let selected_media_source_path = completion.selected_media_source_path;
        let session_playlist_projected = completion.session_playlist_projected;
        let session_success = completion.session_success;
        let session_error = completion.session_error;
        let selected_media_sync =
            if session_playlist_projected && self.pending_playlist_source_resolution.is_none() {
                self.open_selected_playlist_media_after_shared_playlist_projection(
                    projected_state,
                    selected_playlist_index,
                    selected_media_source_path.clone(),
                )
            } else {
                SelectedPlaylistMediaSyncOutcome::NoChange
            };
        if selected_media_sync.selection_started()
            && let Some(session) = self.session.as_mut()
        {
            session.note_local_playlist_index_reset_intent(true);
        }
        let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
            self.session
                .as_ref()
                .is_some_and(|session| session.has_pending_playlist_index_reset_intent()),
        );
        self.apply_pending_playlist_index_reset_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );
        self.sync_session_playstate_to_attached_player_impl(
            projected_state,
            selection_handoff_ready,
        );

        let mut actions = Vec::new();
        if session_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::Room));
        }
        if session_success {
            let message = Self::shared_playlist_open_success_message(&dispatch, opened_entry_count);
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        if let Some(error) = session_error {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
        }
        handle.push_actions(actions);
    }

    pub(in crate::app::runtime_owner) fn resume_pending_shared_playlist_open_if_ready(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        if self
            .pending_shared_playlist_open
            .as_ref()
            .is_some_and(|pending| !pending.delivery_fence_reached())
        {
            return;
        }
        let Some(pending) = self.pending_shared_playlist_open.take() else {
            return;
        };
        match pending {
            GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. } => {
                self.sync_active_shared_playlist_media_and_playstate_impl(projected_state);
                let _ = self.retry_pending_playlist_source_resolution(handle, projected_state);
            }
            GuiPendingSharedPlaylistOpen::AfterMutation {
                dispatch,
                opened_entry_count,
                selected_playlist_index,
                selected_media_source_path,
                ..
            } => {
                self.finish_shared_playlist_open_after_delivery(
                    handle,
                    projected_state,
                    GuiSharedPlaylistOpenCompletion {
                        dispatch,
                        opened_entry_count,
                        selected_playlist_index,
                        selected_media_source_path,
                        session_playlist_projected: true,
                        session_success: true,
                        session_error: None,
                    },
                );
                let _ = self.retry_pending_playlist_source_resolution(handle, projected_state);
            }
        }
    }

    pub(in crate::app::runtime_owner) fn open_stream_helper_install_location_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        install_location: PathBuf,
    ) {
        if let Err(error) = fs::create_dir_all(&install_location) {
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                format!(
                    "Could not prepare the managed stream-helper install location '{}': {error}",
                    install_location.display()
                ),
            );
            return;
        }
        if let Err(error) = Self::open_system_folder(
            &install_location,
            "the managed stream-helper install location",
        ) {
            Self::push_runtime_error_notification(handle, projected_state, error);
        }
    }
}
