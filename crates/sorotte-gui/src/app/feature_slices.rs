//! Feature-owned projections shared with the application runtime.
//!
//! The shell intentionally contains transient UI details such as navigation,
//! modal and edit state.  The runtime must not receive that entire aggregate.
//! These views are the compatibility boundary while the remaining shell
//! actions are moved into feature reducers.

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
    Updates,
}

/// Typed application command used by the runtime queue.
///
/// `GuiRuntimeRequest` remains the compatibility action façade at call sites;
/// requests are classified once when they cross into the application layer.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiClientCommand {
    feature: GuiFeature,
    request: GuiRuntimeRequest,
}

impl GuiClientCommand {
    pub(super) fn from_compatibility_request(request: GuiRuntimeRequest) -> Self {
        use GuiRuntimeRequest as Request;

        let feature = match &request {
            Request::CheckForUpdates { .. }
            | Request::DownloadUpdate(_)
            | Request::DownloadAndInstallUpdate(_)
            | Request::ApplyStagedUpdate(_) => GuiFeature::Updates,
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
        };
        Self { feature, request }
    }

    pub(super) fn feature(&self) -> GuiFeature {
        self.feature
    }

    pub(super) fn into_compatibility_request(self) -> GuiRuntimeRequest {
        self.request
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

    #[derive(Debug, Clone, PartialEq)]
    pub(super) struct RuntimeView {
        pub(super) model: GuiUpdateCheckState,
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
        assert_eq!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::SetRoom(
                "room".to_owned(),
            ))
            .feature(),
            GuiFeature::Session,
        );
        assert_eq!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::ShuffleEntirePlaylist,)
                .feature(),
            GuiFeature::Playlist,
        );
        assert_eq!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::StartPlexAuth)
                .feature(),
            GuiFeature::Plex,
        );
        assert_eq!(
            GuiClientCommand::from_compatibility_request(GuiRuntimeRequest::CheckForUpdates {
                language: "en".to_owned(),
                update_channel: None,
                user_initiated: true,
            })
            .feature(),
            GuiFeature::Updates,
        );
    }
}
