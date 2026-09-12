//! Feature-owned projections shared with the application runtime.
//!
//! The shell intentionally contains transient UI details such as navigation,
//! modal and edit state.  The runtime must not receive that entire aggregate.
//! These views carry feature state between the shell and runtime.

use super::remote_services;
#[cfg(test)]
use super::runtime_bridge::GuiPlexPlaylistJobCancellationReason;
use super::runtime_bridge::GuiRuntimeRequest;
use super::runtime_state::GuiRuntimeState;
use super::shell_state::{
    FirstRunConfigurationDialogDraft, GuiCommandAvailabilityRuntimeOverride,
    GuiCommandAvailabilityState, GuiConfigStorageChangeTarget, GuiConfigStorageRuntimeSnapshot,
    GuiMediaIndexStatusState, GuiMediaMatchRemediationState, GuiMediaMatchState,
    GuiPendingOperationState, GuiPlayerSetupIssue, GuiPlexPlaylistSearchState, GuiPlexState,
    GuiPluginEnablementState, GuiSavedServerConnectIntent, GuiSeekPreparationDegradedReason,
    GuiSeekPreparationState, GuiSelectionState, GuiStreamHelperRemediationState,
    GuiStreamHelperState, GuiValidationIssue, GuiValidationState, MainWindowShellState,
    MediaSearchWorkflowShellState, MenuActionRuntimeOverride, MenuDialogShellState,
    PublicServerBrowserShellState, SorotteGuiShellAppState,
};
use super::ui_state::GuiUpdateCheckState;
use sorotte_client_app::app_boundary::{commands::LocalOffsetCommand, state::StoredClientSettings};

/// Feature routing for commands sent from the shell to the application layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiFeature {
    Session,
    Playlist,
    MediaResolution,
    MediaMatch,
    Plex,
    Settings,
}

/// Typed application command used by the runtime queue.
///
/// `GuiRuntimeRequest` remains the shell action façade at call sites;
/// requests are classified once when they cross into the application layer.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiClientCommand {
    Player(player::Command),
    Updates(Box<updates::Command>),
    Routed {
        feature: GuiFeature,
        request: Box<GuiRuntimeRequest>,
    },
}

impl GuiClientCommand {
    pub(in crate::app) fn from_runtime_request(request: GuiRuntimeRequest) -> Self {
        use GuiRuntimeRequest as Request;

        match request {
            Request::UndoSeek => Self::Player(player::Command::UndoSeek),
            Request::SetOffset(command) => Self::Player(player::Command::SetOffset(command)),
            Request::SetAutoplayEnabled(enabled) => {
                Self::Player(player::Command::SetAutoplayEnabled(enabled))
            }
            Request::SetAutoplayThreshold(threshold) => {
                Self::Player(player::Command::SetAutoplayThreshold(threshold))
            }
            Request::RetryPlayerLaunch => Self::Player(player::Command::RetryLaunch),
            Request::RetryPlayerSettings => Self::Player(player::Command::RetrySettings),
            Request::RetryChatOsdIntegration => {
                Self::Player(player::Command::RetryChatOsdIntegration)
            }
            Request::SeekOffset(offset_seconds) => {
                Self::Player(player::Command::SeekOffset(offset_seconds))
            }
            Request::SeekToPosition(position_seconds) => {
                Self::Player(player::Command::SeekToPosition(position_seconds))
            }
            Request::KeepWaitingForSeekPreparation => {
                Self::Player(player::Command::KeepWaitingForSeekPreparation)
            }
            Request::CancelSeekPreparation => Self::Player(player::Command::CancelSeekPreparation),
            Request::JoinNearestBufferedSeekPreparation => {
                Self::Player(player::Command::JoinNearestBufferedSeekPreparation)
            }
            Request::SetPlaybackPaused(paused) => Self::Player(player::Command::SetPaused(paused)),
            Request::TogglePlaybackPause => Self::Player(player::Command::TogglePause),
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
            request => Self::Routed {
                feature: Self::request_feature(&request),
                request: Box::new(request),
            },
        }
    }

    fn request_feature(request: &GuiRuntimeRequest) -> GuiFeature {
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
            | Request::ImportSharedPlaylistFile { .. }
            | Request::OpenMainWindowUserMedia(_)
            | Request::OpenMainWindowUserContainingFolder(_)
            | Request::RetryPendingStreamMediaOpen => GuiFeature::MediaResolution,
            Request::UndoSeek
            | Request::SetOffset(_)
            | Request::SetAutoplayEnabled(_)
            | Request::SetAutoplayThreshold(_)
            | Request::RetryPlayerLaunch
            | Request::RetryPlayerSettings
            | Request::RetryChatOsdIntegration
            | Request::SeekOffset(_)
            | Request::SeekToPosition(_)
            | Request::KeepWaitingForSeekPreparation
            | Request::CancelSeekPreparation
            | Request::JoinNearestBufferedSeekPreparation
            | Request::SetPlaybackPaused(_)
            | Request::TogglePlaybackPause => {
                unreachable!("player requests are converted to typed player commands")
            }
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
            | Request::ResolvePlexPlaylistItem { .. }
            | Request::CancelPlexPlaylistJobs { .. } => GuiFeature::Plex,
            Request::SetPluginEnabled { .. }
            | Request::InstallStreamHelper
            | Request::IntegrateStreamHelperDownloader(_)
            | Request::IntegrateStreamHelperJsRuntime(_)
            | Request::OpenStreamHelperInstallLocation
            | Request::RecheckStreamHelper => GuiFeature::Settings,
        }
    }

    pub(in crate::app) fn into_runtime_request(self) -> GuiRuntimeRequest {
        match self {
            Self::Player(command) => command.into_runtime_request(),
            Self::Updates(command) => (*command).into_runtime_request(),
            Self::Routed { request, .. } => *request,
        }
    }
}

pub(in crate::app) mod session {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) commands: GuiCommandAvailabilityState,
        pub(in crate::app) command_overrides: GuiCommandAvailabilityRuntimeOverride,
        pub(in crate::app) menu_overrides: Vec<MenuActionRuntimeOverride>,
        pub(in crate::app) menus: MenuDialogShellState,
        pub(in crate::app) pending_operation: Option<GuiPendingOperationState>,
        pub(in crate::app) pending_local_ready_target: Option<bool>,
        pub(in crate::app) pending_saved_server_connect_intent: Option<GuiSavedServerConnectIntent>,
        pub(in crate::app) outgoing_chat_message: Option<String>,
        pub(in crate::app) public_servers: PublicServerBrowserShellState,
    }
}

pub(in crate::app) mod player {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) enum Command {
        UndoSeek,
        SetOffset(LocalOffsetCommand),
        SetAutoplayEnabled(bool),
        SetAutoplayThreshold(usize),
        RetryLaunch,
        RetrySettings,
        RetryChatOsdIntegration,
        SeekOffset(f64),
        SeekToPosition(f64),
        KeepWaitingForSeekPreparation,
        CancelSeekPreparation,
        JoinNearestBufferedSeekPreparation,
        SetPaused(bool),
        TogglePause,
    }

    impl Command {
        pub(in crate::app) fn into_runtime_request(self) -> GuiRuntimeRequest {
            match self {
                Self::UndoSeek => GuiRuntimeRequest::UndoSeek,
                Self::SetOffset(command) => GuiRuntimeRequest::SetOffset(command),
                Self::SetAutoplayEnabled(enabled) => GuiRuntimeRequest::SetAutoplayEnabled(enabled),
                Self::SetAutoplayThreshold(threshold) => {
                    GuiRuntimeRequest::SetAutoplayThreshold(threshold)
                }
                Self::RetryLaunch => GuiRuntimeRequest::RetryPlayerLaunch,
                Self::RetrySettings => GuiRuntimeRequest::RetryPlayerSettings,
                Self::RetryChatOsdIntegration => GuiRuntimeRequest::RetryChatOsdIntegration,
                Self::SeekOffset(offset_seconds) => GuiRuntimeRequest::SeekOffset(offset_seconds),
                Self::SeekToPosition(position_seconds) => {
                    GuiRuntimeRequest::SeekToPosition(position_seconds)
                }
                Self::KeepWaitingForSeekPreparation => {
                    GuiRuntimeRequest::KeepWaitingForSeekPreparation
                }
                Self::CancelSeekPreparation => GuiRuntimeRequest::CancelSeekPreparation,
                Self::JoinNearestBufferedSeekPreparation => {
                    GuiRuntimeRequest::JoinNearestBufferedSeekPreparation
                }
                Self::SetPaused(paused) => GuiRuntimeRequest::SetPlaybackPaused(paused),
                Self::TogglePause => GuiRuntimeRequest::TogglePlaybackPause,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) setup_issue: Option<GuiPlayerSetupIssue>,
        pub(in crate::app) seek_preparation: Option<GuiSeekPreparationState>,
        pub(in crate::app) seek_preparation_degraded_reason:
            Option<GuiSeekPreparationDegradedReason>,
        pub(in crate::app) stream_helper: GuiStreamHelperState,
        pub(in crate::app) stream_helper_remediation: GuiStreamHelperRemediationState,
    }
}

pub(in crate::app) mod playlist {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) main_window: MainWindowShellState,
        pub(in crate::app) selection: GuiSelectionState,
        pub(in crate::app) selection_is_local: bool,
        pub(in crate::app) undo_snapshot: Option<Vec<String>>,
        pub(in crate::app) source_undo_snapshot:
            Option<Vec<super::super::shell_state::GuiPlaylistSourceState>>,
        pub(in crate::app) entry_id_undo_snapshot:
            Option<Vec<super::super::shell_state::GuiPlaylistEntryId>>,
        pub(in crate::app) shuffle_nonce: u64,
    }
}

pub(in crate::app) mod media_resolution {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) index_status: GuiMediaIndexStatusState,
        pub(in crate::app) search: MediaSearchWorkflowShellState,
        pub(in crate::app) last_dialog_directory: Option<String>,
    }
}

pub(in crate::app) mod media_match {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) model: GuiMediaMatchState,
        pub(in crate::app) remediation: GuiMediaMatchRemediationState,
    }
}

pub(in crate::app) mod plex {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) model: GuiPlexState,
        pub(in crate::app) playlist_search: Option<GuiPlexPlaylistSearchState>,
    }
}

pub(in crate::app) mod settings {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::app) struct RuntimeView {
        pub(in crate::app) active_application_language: Option<String>,
        pub(in crate::app) active_application_force_gui_prompt: Option<bool>,
        pub(in crate::app) plugin_enablement: GuiPluginEnablementState,
        pub(in crate::app) config_storage: GuiConfigStorageRuntimeSnapshot,
        pub(in crate::app) pending_storage_target: Option<GuiConfigStorageChangeTarget>,
        pub(in crate::app) saved: StoredClientSettings,
        pub(in crate::app) draft: FirstRunConfigurationDialogDraft,
        pub(in crate::app) validation: GuiValidationState,
        pub(in crate::app) runtime_validation_issues: Vec<GuiValidationIssue>,
    }
}

pub(in crate::app) mod updates {
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
        pub(in crate::app) fn into_runtime_request(self) -> GuiRuntimeRequest {
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
pub(in crate::app) struct GuiRuntimeInput {
    state: GuiRuntimeState,
}

impl GuiRuntimeInput {
    pub(in crate::app) fn from_shell(state: &SorotteGuiShellAppState) -> Self {
        Self {
            state: GuiRuntimeState {
                session: session::RuntimeView {
                    commands: state.commands.clone(),
                    command_overrides: state.runtime_command_availability_override.clone(),
                    menu_overrides: state.runtime_menu_action_overrides.clone(),
                    menus: state.menus.clone(),
                    pending_operation: state.pending_operation.clone(),
                    pending_local_ready_target: state.pending_local_ready_target,
                    pending_saved_server_connect_intent: state.pending_saved_server_connect_intent,
                    outgoing_chat_message: state.outgoing_chat_message.clone(),
                    public_servers: state.public_servers.clone(),
                },
                player: player::RuntimeView {
                    setup_issue: state.player_setup_issue.clone(),
                    seek_preparation: state.seek_preparation.clone(),
                    seek_preparation_degraded_reason: state.seek_preparation_degraded_reason,
                    stream_helper: state.stream_helper.clone(),
                    stream_helper_remediation: state.stream_helper_remediation.clone(),
                },
                playlist: playlist::RuntimeView {
                    main_window: state.main_window.clone(),
                    selection: state.selection.clone(),
                    selection_is_local: state.main_window_playlist_selection_is_local,
                    undo_snapshot: state.playlist_undo_snapshot.clone(),
                    source_undo_snapshot: state.playlist_source_undo_snapshot.clone(),
                    entry_id_undo_snapshot: state.playlist_entry_id_undo_snapshot.clone(),
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
                    active_application_language: state.active_application_language.clone(),
                    active_application_force_gui_prompt: state.active_application_force_gui_prompt,
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
                        automatic: state.saved_configuration.check_for_updates_automatically
                            == Some(true),
                        last_checked_for_updates: state
                            .saved_configuration
                            .last_checked_for_updates
                            .clone(),
                        language: state.update_check_language(),
                        channel: state.update_check_channel(),
                    },
                },
            },
        }
    }

    pub(in crate::app) fn matches_shell(&self, state: &SorotteGuiShellAppState) -> bool {
        self.state.session.commands == state.commands
            && self.state.session.command_overrides == state.runtime_command_availability_override
            && self.state.session.menu_overrides == state.runtime_menu_action_overrides
            && self.state.session.menus == state.menus
            && self.state.session.pending_operation == state.pending_operation
            && self.state.session.pending_local_ready_target == state.pending_local_ready_target
            && self.state.session.pending_saved_server_connect_intent
                == state.pending_saved_server_connect_intent
            && self.state.session.outgoing_chat_message == state.outgoing_chat_message
            && self.state.session.public_servers == state.public_servers
            && self.state.player.setup_issue == state.player_setup_issue
            && self.state.player.seek_preparation == state.seek_preparation
            && self.state.player.seek_preparation_degraded_reason
                == state.seek_preparation_degraded_reason
            && self.state.player.stream_helper == state.stream_helper
            && self.state.player.stream_helper_remediation == state.stream_helper_remediation
            && self.state.playlist.main_window == state.main_window
            && self.state.playlist.selection == state.selection
            && self.state.playlist.selection_is_local
                == state.main_window_playlist_selection_is_local
            && self.state.playlist.undo_snapshot == state.playlist_undo_snapshot
            && self.state.playlist.source_undo_snapshot == state.playlist_source_undo_snapshot
            && self.state.playlist.entry_id_undo_snapshot == state.playlist_entry_id_undo_snapshot
            && self.state.playlist.shuffle_nonce == state.playlist_shuffle_nonce
            && self.state.media_resolution.index_status == state.media_index_status
            && self.state.media_resolution.search == state.media_search
            && self.state.media_resolution.last_dialog_directory
                == state.last_media_dialog_directory
            && self.state.media_match.model == state.media_match
            && self.state.media_match.remediation == state.media_match_remediation
            && self.state.plex.model == state.plex
            && self.state.plex.playlist_search == state.plex_playlist_search
            && self.state.settings.active_application_language == state.active_application_language
            && self.state.settings.active_application_force_gui_prompt
                == state.active_application_force_gui_prompt
            && self.state.settings.plugin_enablement == state.plugin_enablement
            && self.state.settings.config_storage == state.config_storage
            && self.state.settings.pending_storage_target == state.pending_config_storage_target
            && self.state.settings.saved == state.saved_configuration
            && self.state.settings.draft == state.configuration
            && self.state.settings.validation == state.validation
            && self.state.settings.runtime_validation_issues == state.runtime_validation_issues
            && self.state.updates.model == state.update_check
            && self.state.updates.policy.automatic
                == (state.saved_configuration.check_for_updates_automatically == Some(true))
            && self.state.updates.policy.last_checked_for_updates
                == state.saved_configuration.last_checked_for_updates
            && self.state.updates.policy.language == state.update_check_language()
            && self.state.updates.policy.channel == state.update_check_channel()
    }

    pub(in crate::app) fn to_runtime_state(&self) -> GuiRuntimeState {
        self.state.clone()
    }

    pub(in crate::app) fn updates(&self) -> &updates::RuntimeView {
        &self.state.updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shell_state::{GuiConfigurationTab, GuiShellModal, GuiShellView};

    #[test]
    fn runtime_input_ignores_ui_only_navigation_modal_and_edit_state() {
        let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
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
    fn runtime_state_preserves_supplied_feature_values() {
        let mut state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
        state.pending_local_ready_target = Some(true);
        state.playlist_shuffle_nonce = 42;
        state.last_media_dialog_directory = Some("C:/media".to_owned());

        let input = GuiRuntimeInput::from_shell(&state);
        let projected = input.to_runtime_state();
        assert_eq!(projected.session.pending_local_ready_target, Some(true));
        assert_eq!(projected.playlist.shuffle_nonce, 42);
        assert_eq!(
            projected.media_resolution.last_dialog_directory.as_deref(),
            Some("C:/media")
        );
    }

    #[test]
    fn runtime_commands_are_routed_to_feature_owners() {
        assert!(matches!(
            GuiClientCommand::from_runtime_request(GuiRuntimeRequest::SetRoom("room".to_owned(),)),
            GuiClientCommand::Routed {
                feature: GuiFeature::Session,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_runtime_request(GuiRuntimeRequest::ShuffleEntirePlaylist,),
            GuiClientCommand::Routed {
                feature: GuiFeature::Playlist,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_runtime_request(GuiRuntimeRequest::StartPlexAuth),
            GuiClientCommand::Routed {
                feature: GuiFeature::Plex,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_runtime_request(GuiRuntimeRequest::CancelPlexPlaylistJobs {
                reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
            },),
            GuiClientCommand::Routed {
                feature: GuiFeature::Plex,
                ..
            }
        ));
        assert!(matches!(
            GuiClientCommand::from_runtime_request(GuiRuntimeRequest::CheckForUpdates {
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
            stage_lease: None,
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
            let command = GuiClientCommand::from_runtime_request(request.clone());
            assert!(matches!(command, GuiClientCommand::Updates(_)));
            assert_eq!(command.into_runtime_request(), request);
        }
    }

    #[test]
    fn every_player_request_uses_the_typed_player_route_and_round_trips() {
        let requests = vec![
            GuiRuntimeRequest::UndoSeek,
            GuiRuntimeRequest::SetOffset(LocalOffsetCommand::Relative(2.5)),
            GuiRuntimeRequest::SetAutoplayEnabled(true),
            GuiRuntimeRequest::SetAutoplayThreshold(3),
            GuiRuntimeRequest::RetryPlayerLaunch,
            GuiRuntimeRequest::RetryPlayerSettings,
            GuiRuntimeRequest::SeekOffset(-5.0),
            GuiRuntimeRequest::SeekToPosition(42.0),
            GuiRuntimeRequest::KeepWaitingForSeekPreparation,
            GuiRuntimeRequest::CancelSeekPreparation,
            GuiRuntimeRequest::JoinNearestBufferedSeekPreparation,
            GuiRuntimeRequest::SetPlaybackPaused(true),
            GuiRuntimeRequest::TogglePlaybackPause,
        ];

        for request in requests {
            let command = GuiClientCommand::from_runtime_request(request.clone());
            assert!(matches!(command, GuiClientCommand::Player(_)));
            assert_eq!(command.into_runtime_request(), request);
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
            stage_lease: None,
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
