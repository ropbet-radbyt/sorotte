//! Feature-owned projections shared with the application runtime.
//!
//! The shell intentionally contains transient UI details such as navigation,
//! modal and edit state.  The runtime must not receive that entire aggregate.
//! These views are the compatibility boundary while the remaining shell
//! actions are moved into feature reducers.

use super::remote_services;
use super::runtime_bridge::GuiRuntimeRequest;
use super::shell_state::{
    FirstRunConfigurationDialogDraft, GuiCommandAvailabilityRuntimeOverride,
    GuiCommandAvailabilityState, GuiConfigStorageChangeTarget, GuiConfigStorageRuntimeSnapshot,
    GuiMediaIndexStatusState, GuiMediaMatchRemediationState, GuiMediaMatchState,
    GuiPendingOperationState, GuiPlayerSetupIssue, GuiPlexPlaylistSearchState, GuiPlexState,
    GuiPluginEnablementState, GuiSelectionState, GuiStreamHelperRemediationState,
    GuiStreamHelperState, GuiValidationIssue, GuiValidationState, MainWindowShellState,
    MediaSearchWorkflowShellState, MenuActionRuntimeOverride, MenuDialogShellState,
    PublicServerBrowserShellState, SorotteGuiShellAppState,
};
use super::ui_state::GuiUpdateCheckState;
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

/// Feature routing for commands sent from the shell to the application layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiFeature {
    Session,
    Player,
    Playlist,
    MediaResolution,
    MediaMatch,
    Plex,
    Settings,
}

/// Typed application command used by the runtime queue.
///
/// `GuiRuntimeRequest` remains the compatibility action façade at call sites;
/// requests are classified once when they cross into the application layer.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiClientCommand {
    Updates(Box<updates::Command>),
    Legacy {
        feature: GuiFeature,
        request: Box<GuiRuntimeRequest>,
    },
}

impl GuiClientCommand {
    pub(super) fn from_compatibility_request(request: GuiRuntimeRequest) -> Self {
        use GuiRuntimeRequest as Request;

        match request {
            Request::CheckForUpdates {
                language,
                update_channel,
                user_initiated,
            } => Self::Updates(Box::new(updates::Command::CheckForUpdates {
                language,
                update_channel,
                user_initiated,
            })),
            Request::DownloadUpdate(candidate) => {
                Self::Updates(Box::new(updates::Command::Download(candidate)))
            }
            Request::DownloadAndInstallUpdate(candidate) => {
                Self::Updates(Box::new(updates::Command::DownloadAndInstall(candidate)))
            }
            Request::ApplyStagedUpdate(staged_update) => {
                Self::Updates(Box::new(updates::Command::ApplyStaged(staged_update)))
            }
            request => Self::Legacy {
                feature: Self::legacy_feature(&request),
                request: Box::new(request),
            },
        }
    }

    fn legacy_feature(request: &GuiRuntimeRequest) -> GuiFeature {
        use GuiRuntimeRequest as Request;

        match request {
            Request::CheckForUpdates { .. }
            | Request::DownloadUpdate(_)
            | Request::DownloadAndInstallUpdate(_)
            | Request::ApplyStagedUpdate(_) => {
                unreachable!("update requests are converted to typed update commands")
            }
            Request::SetRoom(_)
            | Request::ReturnToDefaultRoom
            | Request::SetLocalReady(_)
            | Request::SetReadyForUser { .. }
            | Request::RequestControllerAuth { .. }
            | Request::SendChatMessage(_)
            | Request::CompletePendingOperation(_)
            | Request::CancelPendingOperation(_) => GuiFeature::Session,
            Request::OpenMediaFiles { .. }
            | Request::OpenMainWindowUserMedia(_)
            | Request::OpenMainWindowUserContainingFolder(_)
            | Request::RetryPendingStreamMediaOpen => GuiFeature::MediaResolution,
            Request::UndoSeek
            | Request::SetOffset(_)
            | Request::SetAutoplayEnabled(_)
            | Request::SetAutoplayThreshold(_)
            | Request::RetryPlayerLaunch
            | Request::SeekOffset(_)
            | Request::SeekToPosition(_)
            | Request::SetPlaybackPaused(_)
            | Request::TogglePlaybackPause => GuiFeature::Player,
            Request::QueuePlaylistEntry { .. }
            | Request::SetPlaylistIndex(_)
            | Request::DeletePlaylistIndex(_)
            | Request::UndoPlaylistChange
            | Request::ShuffleRemainingPlaylist
            | Request::ShuffleEntirePlaylist
            | Request::ReplacePlaylist { .. }
            | Request::ResolvePlaylistSource { .. }
            | Request::AdvancePlaylistIndex => GuiFeature::Playlist,
            Request::InstallMediaMatchTools
            | Request::ImportMediaMatchFfmpeg(_)
            | Request::ImportMediaMatchFfprobe(_)
            | Request::OpenMediaMatchInstallLocation
            | Request::RecheckMediaMatchTools
            | Request::RebuildMediaMatchIndex
            | Request::CancelMediaMatchRebuild
            | Request::ClearMediaMatchCache
            | Request::SetMediaMatchFingerprintingEnabled(_)
            | Request::SetMediaMatchBackgroundWarmupEnabled(_)
            | Request::SetMediaMatchWireSharingEnabled(_)
            | Request::SetMediaMatchRuntimeToleranceEnabled(_)
            | Request::SetMediaMatchAutoplayPolicy(_) => GuiFeature::MediaMatch,
            Request::StartPlexAuth
            | Request::PollPlexAuth
            | Request::RefreshPlexServers
            | Request::SelectPlexServer { .. }
            | Request::TogglePlexSync(_)
            | Request::TogglePlexStreaming(_)
            | Request::DisconnectPlex
            | Request::SearchSelectedPlexServerMedia { .. }
            | Request::ResolvePlexPlaylistItem { .. } => GuiFeature::Plex,
            Request::SetPluginEnabled { .. }
            | Request::InstallStreamHelper
            | Request::IntegrateStreamHelperDownloader(_)
            | Request::IntegrateStreamHelperJsRuntime(_)
            | Request::OpenStreamHelperInstallLocation
            | Request::RecheckStreamHelper => GuiFeature::Settings,
        }
    }

    pub(super) fn into_compatibility_request(self) -> GuiRuntimeRequest {
        match self {
            Self::Updates(command) => (*command).into_compatibility_request(),
            Self::Legacy { request, .. } => *request,
        }
    }
}

pub(super) mod session {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) commands: GuiCommandAvailabilityState,
        pub(super) command_overrides: GuiCommandAvailabilityRuntimeOverride,
        pub(super) menu_overrides: Vec<MenuActionRuntimeOverride>,
        pub(super) menus: MenuDialogShellState,
        pub(super) pending_operation: Option<GuiPendingOperationState>,
        pub(super) pending_local_ready_target: Option<bool>,
        pub(super) pending_saved_server_connect_saves_configuration: bool,
        pub(super) outgoing_chat_message: Option<String>,
        pub(super) public_servers: PublicServerBrowserShellState,
    }
}

pub(super) mod player {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) setup_issue: Option<GuiPlayerSetupIssue>,
        pub(super) stream_helper: GuiStreamHelperState,
        pub(super) stream_helper_remediation: GuiStreamHelperRemediationState,
    }
}

pub(super) mod playlist {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) main_window: MainWindowShellState,
        pub(super) selection: GuiSelectionState,
        pub(super) selection_is_local: bool,
        pub(super) undo_snapshot: Option<Vec<String>>,
        pub(super) source_undo_snapshot:
            Option<Vec<super::super::shell_state::GuiPlaylistSourceState>>,
        pub(super) shuffle_nonce: u64,
    }
}

pub(super) mod media_resolution {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) index_status: GuiMediaIndexStatusState,
        pub(super) search: MediaSearchWorkflowShellState,
        pub(super) last_dialog_directory: Option<String>,
    }
}

pub(super) mod media_match {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) model: GuiMediaMatchState,
        pub(super) remediation: GuiMediaMatchRemediationState,
    }
}

pub(super) mod plex {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) model: GuiPlexState,
        pub(super) playlist_search: Option<GuiPlexPlaylistSearchState>,
    }
}

pub(super) mod settings {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) plugin_enablement: GuiPluginEnablementState,
        pub(super) config_storage: GuiConfigStorageRuntimeSnapshot,
        pub(super) pending_storage_target: Option<GuiConfigStorageChangeTarget>,
        pub(super) saved: StoredClientSettingsMvp,
        pub(super) draft: FirstRunConfigurationDialogDraft,
        pub(super) validation: GuiValidationState,
        pub(super) runtime_validation_issues: Vec<GuiValidationIssue>,
    }
}

pub(super) mod updates {
    use super::*;

    #[derive(Clone, PartialEq)]
    pub(in crate::app) enum Command {
        CheckForUpdates {
            language: String,
            update_channel: Option<String>,
            user_initiated: bool,
        },
        Download(remote_services::UpdateCandidate),
        DownloadAndInstall(remote_services::UpdateCandidate),
        ApplyStaged(remote_services::StagedUpdate),
    }

    impl std::fmt::Debug for Command {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::CheckForUpdates {
                    language,
                    update_channel,
                    user_initiated,
                } => formatter
                    .debug_struct("CheckForUpdates")
                    .field("language", language)
                    .field("update_channel", update_channel)
                    .field("user_initiated", user_initiated)
                    .finish(),
                Self::Download(_) => formatter
                    .debug_tuple("Download")
                    .field(&"<redacted>")
                    .finish(),
                Self::DownloadAndInstall(_) => formatter
                    .debug_tuple("DownloadAndInstall")
                    .field(&"<redacted>")
                    .finish(),
                Self::ApplyStaged(_) => formatter
                    .debug_tuple("ApplyStaged")
                    .field(&"<redacted>")
                    .finish(),
            }
        }
    }

    impl Command {
        pub(super) fn into_compatibility_request(self) -> GuiRuntimeRequest {
            match self {
                Self::CheckForUpdates {
                    language,
                    update_channel,
                    user_initiated,
                } => GuiRuntimeRequest::CheckForUpdates {
                    language,
                    update_channel,
                    user_initiated,
                },
                Self::Download(candidate) => GuiRuntimeRequest::DownloadUpdate(candidate),
                Self::DownloadAndInstall(candidate) => {
                    GuiRuntimeRequest::DownloadAndInstallUpdate(candidate)
                }
                Self::ApplyStaged(staged_update) => {
                    GuiRuntimeRequest::ApplyStagedUpdate(staged_update)
                }
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(in crate::app) struct RuntimePolicy {
        pub(in crate::app) automatic: bool,
        pub(in crate::app) last_checked_for_updates: Option<String>,
        pub(in crate::app) language: String,
        pub(in crate::app) channel: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) model: GuiUpdateCheckState,
        pub(in crate::app) policy: RuntimePolicy,
    }
}

/// The compact input submitted to the runtime worker.
///
/// This deliberately has no navigation, modal, edit-session, notification or
/// browser-only fields. Equality therefore also acts as runtime invalidation:
/// changing UI-only state does not allocate or submit another worker snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiRuntimeInput {
    session: session::RuntimeView,
    player: player::RuntimeView,
    playlist: playlist::RuntimeView,
    media_resolution: media_resolution::RuntimeView,
    media_match: media_match::RuntimeView,
    plex: plex::RuntimeView,
    settings: settings::RuntimeView,
    updates: updates::RuntimeView,
}

impl GuiRuntimeInput {
    pub(super) fn from_shell(state: &SorotteGuiShellAppState) -> Self {
        Self {
            session: session::RuntimeView {
                commands: state.commands.clone(),
                command_overrides: state.runtime_command_availability_override.clone(),
                menu_overrides: state.runtime_menu_action_overrides.clone(),
                menus: state.menus.clone(),
                pending_operation: state.pending_operation.clone(),
                pending_local_ready_target: state.pending_local_ready_target,
                pending_saved_server_connect_saves_configuration: state
                    .pending_saved_server_connect_saves_configuration,
                outgoing_chat_message: state.outgoing_chat_message.clone(),
                public_servers: state.public_servers.clone(),
            },
            player: player::RuntimeView {
                setup_issue: state.player_setup_issue.clone(),
                stream_helper: state.stream_helper.clone(),
                stream_helper_remediation: state.stream_helper_remediation.clone(),
            },
            playlist: playlist::RuntimeView {
                main_window: state.main_window.clone(),
                selection: state.selection.clone(),
                selection_is_local: state.main_window_playlist_selection_is_local,
                undo_snapshot: state.playlist_undo_snapshot.clone(),
                source_undo_snapshot: state.playlist_source_undo_snapshot.clone(),
                shuffle_nonce: state.playlist_shuffle_nonce,
            },
            media_resolution: media_resolution::RuntimeView {
                index_status: state.media_index_status.clone(),
                search: state.media_search.clone(),
                last_dialog_directory: state.last_media_dialog_directory.clone(),
            },
            media_match: media_match::RuntimeView {
                model: state.media_match.clone(),
                remediation: state.media_match_remediation.clone(),
            },
            plex: plex::RuntimeView {
                model: state.plex.clone(),
                playlist_search: state.plex_playlist_search.clone(),
            },
            settings: settings::RuntimeView {
                plugin_enablement: state.plugin_enablement,
                config_storage: state.config_storage.clone(),
                pending_storage_target: state.pending_config_storage_target.clone(),
                saved: state.saved_configuration.clone(),
                draft: state.configuration.clone(),
                validation: state.validation.clone(),
                runtime_validation_issues: state.runtime_validation_issues.clone(),
            },
            updates: updates::RuntimeView {
                model: state.update_check.clone(),
                policy: updates::RuntimePolicy {
                    automatic: state.configuration.settings.check_for_updates_automatically
                        == Some(true),
                    last_checked_for_updates: state
                        .configuration
                        .settings
                        .last_checked_for_updates
                        .clone(),
                    language: state.update_check_language(),
                    channel: state.update_check_channel(),
                },
            },
        }
    }

    pub(super) fn matches_shell(&self, state: &SorotteGuiShellAppState) -> bool {
        self.session.commands == state.commands
            && self.session.command_overrides == state.runtime_command_availability_override
            && self.session.menu_overrides == state.runtime_menu_action_overrides
            && self.session.menus == state.menus
            && self.session.pending_operation == state.pending_operation
            && self.session.pending_local_ready_target == state.pending_local_ready_target
            && self
                .session
                .pending_saved_server_connect_saves_configuration
                == state.pending_saved_server_connect_saves_configuration
            && self.session.outgoing_chat_message == state.outgoing_chat_message
            && self.session.public_servers == state.public_servers
            && self.player.setup_issue == state.player_setup_issue
            && self.player.stream_helper == state.stream_helper
            && self.player.stream_helper_remediation == state.stream_helper_remediation
            && self.playlist.main_window == state.main_window
            && self.playlist.selection == state.selection
            && self.playlist.selection_is_local == state.main_window_playlist_selection_is_local
            && self.playlist.undo_snapshot == state.playlist_undo_snapshot
            && self.playlist.source_undo_snapshot == state.playlist_source_undo_snapshot
            && self.playlist.shuffle_nonce == state.playlist_shuffle_nonce
            && self.media_resolution.index_status == state.media_index_status
            && self.media_resolution.search == state.media_search
            && self.media_resolution.last_dialog_directory == state.last_media_dialog_directory
            && self.media_match.model == state.media_match
            && self.media_match.remediation == state.media_match_remediation
            && self.plex.model == state.plex
            && self.plex.playlist_search == state.plex_playlist_search
            && self.settings.plugin_enablement == state.plugin_enablement
            && self.settings.config_storage == state.config_storage
            && self.settings.pending_storage_target == state.pending_config_storage_target
            && self.settings.saved == state.saved_configuration
            && self.settings.draft == state.configuration
            && self.settings.validation == state.validation
            && self.settings.runtime_validation_issues == state.runtime_validation_issues
            && self.updates.model == state.update_check
            && self.updates.policy.automatic
                == (state.configuration.settings.check_for_updates_automatically == Some(true))
            && self.updates.policy.last_checked_for_updates
                == state.configuration.settings.last_checked_for_updates
            && self.updates.policy.language == state.update_check_language()
            && self.updates.policy.channel == state.update_check_channel()
    }

    /// Builds the temporary shell-shaped projection used by the compatibility
    /// reducer. No UI-owned shell aggregate crosses the thread boundary.
    pub(super) fn to_compatibility_projection(&self) -> SorotteGuiShellAppState {
        let mut state = SorotteGuiShellAppState::from_stored_settings(&self.settings.saved);
        state.commands = self.session.commands.clone();
        state.runtime_command_availability_override = self.session.command_overrides.clone();
        state.runtime_menu_action_overrides = self.session.menu_overrides.clone();
        state.menus = self.session.menus.clone();
        state.pending_operation = self.session.pending_operation.clone();
        state.pending_local_ready_target = self.session.pending_local_ready_target;
        state.pending_saved_server_connect_saves_configuration = self
            .session
            .pending_saved_server_connect_saves_configuration;
        state.outgoing_chat_message = self.session.outgoing_chat_message.clone();
        state.public_servers = self.session.public_servers.clone();

        state.player_setup_issue = self.player.setup_issue.clone();
        state.stream_helper = self.player.stream_helper.clone();
        state.stream_helper_remediation = self.player.stream_helper_remediation.clone();

        state.main_window = self.playlist.main_window.clone();
        state.selection = self.playlist.selection.clone();
        state.main_window_playlist_selection_is_local = self.playlist.selection_is_local;
        state.playlist_undo_snapshot = self.playlist.undo_snapshot.clone();
        state.playlist_source_undo_snapshot = self.playlist.source_undo_snapshot.clone();
        state.playlist_shuffle_nonce = self.playlist.shuffle_nonce;

        state.media_index_status = self.media_resolution.index_status.clone();
        state.media_search = self.media_resolution.search.clone();
        state.last_media_dialog_directory = self.media_resolution.last_dialog_directory.clone();

        state.media_match = self.media_match.model.clone();
        state.media_match_remediation = self.media_match.remediation.clone();
        state.plex = self.plex.model.clone();
        state.plex_playlist_search = self.plex.playlist_search.clone();

        state.plugin_enablement = self.settings.plugin_enablement;
        state.config_storage = self.settings.config_storage.clone();
        state.pending_config_storage_target = self.settings.pending_storage_target.clone();
        state.saved_configuration = self.settings.saved.clone();
        state.configuration = self.settings.draft.clone();
        state.validation = self.settings.validation.clone();
        state.runtime_validation_issues = self.settings.runtime_validation_issues.clone();
        state.update_check = self.updates.model.clone();
        state
    }

    pub(super) fn updates(&self) -> &updates::RuntimeView {
        &self.updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shell_state::{GuiConfigurationTab, GuiShellModal, GuiShellView};

    #[test]
    fn runtime_input_ignores_ui_only_navigation_modal_and_edit_state() {
        let state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
        let expected = GuiRuntimeInput::from_shell(&state);
        let mut ui_only_change = state;
        ui_only_change.active_view = GuiShellView::Room;
        ui_only_change.selected_configuration_tab = GuiConfigurationTab::PrivacyChat;
        ui_only_change.open_modal = Some(GuiShellModal::About);
        ui_only_change.new_main_window_user_draft = "draft user".to_owned();

        assert_eq!(GuiRuntimeInput::from_shell(&ui_only_change), expected);
        assert!(expected.matches_shell(&ui_only_change));
    }

    #[test]
    fn compatibility_projection_preserves_runtime_feature_views() {
        let mut state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
        state.pending_local_ready_target = Some(true);
        state.playlist_shuffle_nonce = 42;
        state.last_media_dialog_directory = Some("C:/media".to_owned());

        let input = GuiRuntimeInput::from_shell(&state);
        let projected = input.to_compatibility_projection();

        assert_eq!(GuiRuntimeInput::from_shell(&projected), input);
    }

    #[test]
    fn compatibility_commands_are_routed_to_feature_owners() {
        assert!(matches!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::SetRoom(
                "room".to_owned(),
            )),
            GuiClientCommand::Legacy {
                feature: GuiFeature::Session,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::ShuffleEntirePlaylist,),
            GuiClientCommand::Legacy {
                feature: GuiFeature::Playlist,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::StartPlexAuth),
            GuiClientCommand::Legacy {
                feature: GuiFeature::Plex,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::CheckForUpdates {
                language: "en".to_owned(),
                update_channel: None,
                user_initiated: true,
            }),
            GuiClientCommand::Updates(command)
                if matches!(command.as_ref(), updates::Command::CheckForUpdates {
                language,
                update_channel: None,
                user_initiated: true,
            } if language == "en")
        ));
    }

    #[test]
    fn every_update_request_uses_the_typed_update_route_and_round_trips() {
        use remote_services::{
            StagedUpdate, UpdateCandidate, UpdateCandidateSource, UpdateChannel,
        };

        let candidate = UpdateCandidate {
            channel: UpdateChannel::Stable,
            version: "1.2.3".to_owned(),
            git_sha: None,
            created_at_utc: String::new(),
            target: "x86_64-pc-windows-msvc".to_owned(),
            package: "sorotte.zip".to_owned(),
            sha256: "abc".to_owned(),
            download_url: "https://example.invalid/sorotte.zip".to_owned(),
            details_url: None,
            source: UpdateCandidateSource::ReleaseAsset,
        };
        let staged = StagedUpdate {
            candidate: candidate.clone(),
            package_path: "package".to_owned(),
            source_dir: "source".to_owned(),
            updater_path: "updater".to_owned(),
            target_exe_path: "target".to_owned(),
            backup_dir: "backup".to_owned(),
            log_path: "log".to_owned(),
            restart: true,
        };
        let requests = vec![
            GuiRuntimeRequest::CheckForUpdates {
                language: "en".to_owned(),
                update_channel: Some("stable".to_owned()),
                user_initiated: true,
            },
            GuiRuntimeRequest::DownloadUpdate(candidate.clone()),
            GuiRuntimeRequest::DownloadAndInstallUpdate(candidate),
            GuiRuntimeRequest::ApplyStagedUpdate(staged),
        ];

        for request in requests {
            let command = GuiClientCommand::from_compatibility_request(request.clone());
            assert!(matches!(command, GuiClientCommand::Updates(_)));
            assert_eq!(command.into_compatibility_request(), request);
        }
    }

    #[test]
    fn typed_update_command_debug_redacts_remote_urls_and_local_stage_paths() {
        use remote_services::{
            StagedUpdate, UpdateCandidate, UpdateCandidateSource, UpdateChannel,
        };

        let marker = "typed-update-secret-marker";
        let candidate = UpdateCandidate {
            channel: UpdateChannel::Stable,
            version: "1.2.3".to_owned(),
            git_sha: None,
            created_at_utc: String::new(),
            target: "x86_64-pc-windows-msvc".to_owned(),
            package: "sorotte.zip".to_owned(),
            sha256: "abc".to_owned(),
            download_url: format!("https://example.invalid/{marker}"),
            details_url: Some(format!("https://example.invalid/details/{marker}")),
            source: UpdateCandidateSource::ReleaseAsset,
        };
        let staged = StagedUpdate {
            candidate: candidate.clone(),
            package_path: format!("C:/updates/{marker}"),
            source_dir: format!("C:/source/{marker}"),
            updater_path: format!("C:/updater/{marker}"),
            target_exe_path: format!("C:/target/{marker}"),
            backup_dir: format!("C:/backup/{marker}"),
            log_path: format!("C:/log/{marker}"),
            restart: true,
        };

        for command in [
            updates::Command::Download(candidate.clone()),
            updates::Command::DownloadAndInstall(candidate),
            updates::Command::ApplyStaged(staged),
        ] {
            let debug = format!("{command:?}");
            assert!(!debug.contains(marker), "debug leaked marker: {debug}");
            assert!(debug.contains("<redacted>"));
        }
    }
}
