use std::{collections::BTreeSet, path::Path, process::Command};

use syncplay_player_api::PlayerAdapter;
use syncplay_player_mpv::LegacySyncplayOsdKind;

use super::super::runtime_bridge::GuiSharedPlaylistOpenDispatch;
use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::{GuiClientCoreChatSessionRuntimeAdapter, GuiOwnedPlayer};
use super::super::shell_state::{
    GuiShellAction, GuiShellView, GuiTransientNotificationLevel, SyncplayGuiShellAppState,
    browser_is_url,
};
use super::super::support::normalized_editable_text;
use super::GuiPersistedConfigRuntimeOwner;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn open_media_unavailable_message_impl(&self, selected_paths: &[String]) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media requires a playback runtime connection; the selected file was not opened."
                .to_owned()
        } else {
            format!(
                "Opening media requires a playback runtime connection; {} selected files were not opened.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_open_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                .to_owned()
        } else {
            format!(
                "Opening media into the shared playlist requires a session or playback runtime connection; {} selected files were not opened or queued.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_session_unavailable_message_impl(&self) -> String {
        "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
            .to_owned()
    }

    fn shared_playlist_entry_for_media_path(path: &str) -> Option<String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains("://") {
            return Some(trimmed.to_owned());
        }
        Some(
            Path::new(trimmed)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(trimmed)
                .to_owned(),
        )
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        let lower_path = path.to_ascii_lowercase();
        if !(lower_path.ends_with(".txt")
            || lower_path.ends_with(".m3u")
            || lower_path.ends_with(".m3u8"))
        {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = contents
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    pub(super) fn shared_playlist_open_dispatch_for_paths_impl(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| Self::shared_playlist_entry_for_media_path(path))
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
            imported_from_file: false,
        })
    }

    fn shared_playlist_open_success_message(dispatch: &GuiSharedPlaylistOpenDispatch) -> String {
        let entry_count = dispatch.playlist_entries.len();
        if dispatch.imported_from_file {
            if entry_count == 1 {
                "Imported 1 entry into the shared playlist.".to_owned()
            } else {
                format!("Imported {entry_count} entries into the shared playlist.")
            }
        } else if entry_count == 1 {
            "Loaded 1 selected media entry into the shared playlist.".to_owned()
        } else {
            format!("Loaded {entry_count} selected media entries into the shared playlist.")
        }
    }

    pub(super) fn seek_unavailable_message_impl(&self, offset_seconds: f64) -> String {
        let base = format!(
            "Playback seek requires a playback runtime connection; the {offset_seconds} second request was not applied."
        );
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn toggle_pause_unavailable_message_impl(&self) -> String {
        let base =
            "Playback toggle requires a playback runtime connection; the pause request was not applied."
                .to_owned();
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn send_chat_unavailable_message_impl(&self) -> String {
        "Chat sending requires a session runtime connection; the message was not sent.".to_owned()
    }

    pub(super) fn push_player_success_impl(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    pub(super) fn push_player_error_impl(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
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
            let player = self.player.as_mut().expect("player should exist");
            (player.name(), player.open_file(&selected_path))
        };
        Some(match open_result {
            Ok(()) => {
                self.player_local_file =
                    Some(Self::placeholder_local_file_for_path(&selected_path));
                self.player_position_seconds = Some(0.0);
                self.refresh_player_state_impl();
                if paths.len() == 1 {
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
            Err(error) => Err(format!(
                "Opening media through the attached {player_name} player failed: {error}"
            )),
        })
    }

    pub(super) fn open_media_files_through_attached_player_impl(
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

    fn selected_shared_playlist_target(state: &SyncplayGuiShellAppState) -> Option<String> {
        if !state.main_window.shared_playlist_enabled {
            return None;
        }

        state
            .selection
            .selected_main_window_playlist
            .and_then(|index| state.main_window.playlist.get(index))
            .and_then(|target| normalized_editable_text(&target.label))
    }

    pub(super) fn current_player_matches_media_target(&self, target: &str) -> bool {
        let Some(local_file) = self.player_local_file.as_ref() else {
            return false;
        };

        if let Some(path) = local_file.path.as_deref() {
            if (cfg!(windows) && path.eq_ignore_ascii_case(target))
                || (!cfg!(windows) && path == target)
            {
                return true;
            }
        }

        let target_name = if browser_is_url(target) {
            Some(target)
        } else {
            Path::new(target).file_name().and_then(|name| name.to_str())
        };
        target_name.is_some_and(|target_name| {
            if cfg!(windows) {
                local_file.name.eq_ignore_ascii_case(target_name)
            } else {
                local_file.name == target_name
            }
        })
    }

    fn resolve_main_window_user_media_target(
        &self,
        state: &SyncplayGuiShellAppState,
        target: &str,
    ) -> Result<Option<String>, String> {
        let Some(target) = normalized_editable_text(target) else {
            return Ok(None);
        };
        if browser_is_url(&target) {
            return Ok(Some(target.to_owned()));
        }

        let target_path = Path::new(&target);
        if target_path.is_file() {
            return Ok(Some(target.to_owned()));
        }

        if let Some(local_path) = self
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
        {
            let local_path = Path::new(local_path);
            let matches_local_file = local_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(&target));
            if matches_local_file && local_path.is_file() {
                return Ok(Some(local_path.to_string_lossy().into_owned()));
            }
            if let Some(parent) = local_path.parent()
                && let Some(found_path) =
                    GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                        &target, parent,
                    )?
            {
                return Ok(Some(found_path));
            }
        }

        let settings = state.configuration.to_stored_settings();
        for directory in settings.media_search_directories.unwrap_or_default() {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(found_path) =
                GuiClientCoreChatSessionRuntimeAdapter::search_path_for_missing_media_target(
                    &target,
                    Path::new(trimmed),
                )?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }

    pub(super) fn sync_selected_shared_playlist_media_to_attached_player_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) {
        let Some(target) = Self::selected_shared_playlist_target(state) else {
            return;
        };

        self.ensure_configured_player_attached();
        if self.player.is_none() {
            return;
        }

        let resolved_target = match self.resolve_main_window_user_media_target(state, &target) {
            Ok(Some(path)) => path,
            Ok(None) | Err(_) => return,
        };
        if self.current_player_matches_media_target(&resolved_target) {
            return;
        }

        let player_paths = [resolved_target];
        let _ = self.open_media_files_through_attached_player_result_impl(&player_paths);
    }

    pub(super) fn open_main_window_user_media_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
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

        self.ensure_configured_player_attached();
        if self.player.is_some() {
            self.open_media_files_through_attached_player_impl(handle, vec![resolved_target]);
        } else {
            Self::push_runtime_unavailable(
                handle,
                self.open_media_unavailable_message_impl(&[resolved_target]),
            );
        }
    }

    fn open_system_file_browser_for_path(path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err(format!(
                "Could not open a containing folder for '{}': no parent directory exists.",
                path.display()
            ));
        };

        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer");
            command.arg(parent);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("open");
            command.arg(parent);
            command
        };
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let mut command = {
            let mut command = Command::new("xdg-open");
            command.arg(parent);
            command
        };

        command.spawn().map_err(|error| {
            format!(
                "Opening the containing folder for '{}' failed: {error}",
                path.display()
            )
        })?;
        Ok(())
    }

    pub(super) fn open_main_window_user_containing_folder_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
        target: String,
    ) {
        let Some(target) = normalized_editable_text(&target) else {
            return;
        };
        let resolved_target =
            match self.resolve_main_window_user_media_target(projected_state, &target) {
                Ok(Some(path)) => path,
                Ok(None) => {
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

    pub(super) fn open_media_files_through_shared_playlist_runtime_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        paths: Vec<String>,
    ) {
        self.ensure_configured_player_attached();
        let selected_paths = paths
            .into_iter()
            .filter_map(|path| normalized_editable_text(&path))
            .collect::<Vec<_>>();
        if selected_paths.is_empty() {
            return;
        }

        let dispatch =
            match Self::shared_playlist_open_dispatch_for_paths_impl(selected_paths.clone()) {
                Ok(dispatch) => dispatch,
                Err(error) => {
                    Self::push_runtime_unavailable(handle, error);
                    return;
                }
            };

        let player_result = dispatch.player_paths.as_ref().and_then(|player_paths| {
            self.open_media_files_through_attached_player_result_impl(player_paths)
        });

        if self.session.is_none() {
            match player_result {
                Some(Ok(message)) => {
                    let warning = self.shared_playlist_session_unavailable_message_impl();
                    handle.push_actions([
                        GuiShellAction::SwitchView(GuiShellView::MainWindow),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Success,
                            message: message.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(message),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Warning,
                            message: warning.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(warning),
                    ]);
                }
                Some(Err(message)) => {
                    let warning = self.shared_playlist_session_unavailable_message_impl();
                    handle.push_actions([
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Warning,
                            message: warning.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(warning),
                        GuiShellAction::PushTransientNotification {
                            level: GuiTransientNotificationLevel::Error,
                            message: message.clone(),
                        },
                        GuiShellAction::AnnounceSystemChatEvent(message),
                    ]);
                }
                None => Self::push_runtime_unavailable(
                    handle,
                    self.shared_playlist_open_unavailable_message_impl(&selected_paths),
                ),
            }
            return;
        }

        let session_result = self
            .session
            .as_mut()
            .expect("session should exist")
            .replace_playlist(
                dispatch.playlist_entries.clone(),
                (!dispatch.playlist_entries.is_empty()).then_some(0),
            );
        let session_success = session_result.is_ok();
        let player_success = player_result.as_ref().is_some_and(Result::is_ok);

        let mut actions = Vec::new();
        if session_success || player_success {
            actions.push(GuiShellAction::SwitchView(GuiShellView::MainWindow));
        }
        if session_success && !player_success {
            let message = Self::shared_playlist_open_success_message(&dispatch);
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message: message.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
        }
        match player_result {
            Some(Ok(message)) => {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: message.clone(),
                });
                actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
            }
            Some(Err(message)) => {
                actions.push(GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: message.clone(),
                });
                actions.push(GuiShellAction::AnnounceSystemChatEvent(message));
            }
            None => {}
        }
        if let Err(error) = session_result {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: error.clone(),
            });
            actions.push(GuiShellAction::AnnounceSystemChatEvent(error));
        }
        handle.push_actions(actions);
    }

    pub(super) fn emit_gui_actions_to_attached_player_impl(&mut self, actions: &[GuiShellAction]) {
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return;
        };
        let mut already_emitted_osd_messages = BTreeSet::new();
        for action in actions {
            match action {
                GuiShellAction::PushChatMessage { sender, message } => {
                    if let Err(error) =
                        player.show_syncplay_legacy_chat_message(&format!("<{sender}> {message}"))
                    {
                        eprintln!(
                            "warning: failed to display GUI chat notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::PushTransientNotification { level, message } => {
                    already_emitted_osd_messages.insert(message.clone());
                    let kind = match level {
                        GuiTransientNotificationLevel::Info
                        | GuiTransientNotificationLevel::Success => {
                            LegacySyncplayOsdKind::Notification
                        }
                        GuiTransientNotificationLevel::Warning
                        | GuiTransientNotificationLevel::Error => LegacySyncplayOsdKind::Alert,
                    };
                    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
                        eprintln!(
                            "warning: failed to display GUI notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if already_emitted_osd_messages.insert(message.clone()) =>
                {
                    if let Err(error) = player
                        .show_syncplay_legacy_message(message, LegacySyncplayOsdKind::Notification)
                    {
                        eprintln!(
                            "warning: failed to display GUI system-chat event via mpv OSD: {error}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn drain_player_chat_input_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        if self.session.is_none() {
            return;
        }

        let mut errors = Vec::new();
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
            let send_result = self
                .session
                .as_mut()
                .expect("session should exist when draining player chat")
                .send_chat_message(message.clone());
            if let Err(error) = send_result {
                errors.push(format!(
                    "Chat input from the attached player could not be sent: {error}"
                ));
            }
        }

        if !errors.is_empty() {
            Self::push_actions_and_project(
                handle,
                projected_state,
                errors
                    .into_iter()
                    .flat_map(|message| {
                        [
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: message.clone(),
                            },
                            GuiShellAction::AnnounceSystemChatEvent(message),
                        ]
                    })
                    .collect(),
            );
        }
    }

    pub(super) fn refresh_player_state_impl(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        while let Some(update) = player.take_playback_telemetry_update() {
            if let Some(paused) = update.paused {
                self.player_paused = Some(paused);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds);
            }
        }
        while let Some(update) = player.take_local_file_update() {
            self.player_local_file = Some(update);
            if self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
    }

    pub(super) fn sync_manual_seek_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_position_seconds: f64,
        target_position_seconds: f64,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(self.player_paused, Some(previous_position_seconds))?;
        let _ = session.record_manual_seek_to_position(target_position_seconds)?;
        session.sync_local_playback_telemetry(self.player_paused, Some(target_position_seconds))?;
        Ok(())
    }

    pub(super) fn sync_playback_pause_into_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
        previous_paused: bool,
        target_paused: bool,
    ) -> Result<(), String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session
            .sync_local_playback_telemetry(Some(previous_paused), self.player_position_seconds)?;
        let _ = session.set_playback_paused(target_paused)?;
        session.sync_local_playback_telemetry(Some(target_paused), self.player_position_seconds)?;
        Ok(())
    }

    pub(super) fn undo_seek_target_position_from_detached_session_impl(
        &mut self,
        state: &SyncplayGuiShellAppState,
    ) -> Result<Option<f64>, String> {
        self.ensure_detached_client_core_chat_session(state)?;
        let Some(session) = self.session.as_mut() else {
            return Ok(None);
        };
        session.sync_local_playback_telemetry(self.player_paused, self.player_position_seconds)?;
        if !session.undo_seek()? {
            return Ok(None);
        }
        let target = session.local_position_seconds();
        session.sync_local_playback_telemetry(self.player_paused, target)?;
        Ok(target)
    }
}
