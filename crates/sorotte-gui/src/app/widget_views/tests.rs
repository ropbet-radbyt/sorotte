use super::{GuiLayoutMode, GuiWidgetRenderer};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiConfigurationTab, GuiDraftRuntimeSnapshot, GuiErrorRuntimeSnapshot,
    GuiMediaIndexRuntimeSnapshot, GuiMediaMatchRemediationRuntimeSnapshot,
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchToolHealth, GuiPlayerSetupIssue,
    GuiPlayerSetupIssueKind, GuiPlayerSetupRuntimeSnapshot, GuiPlexRuntimeSnapshot,
    GuiPlexServerReachability, GuiPlexServerRow, GuiPluginSelection,
    GuiSeekPreparationDegradedReason, GuiSeekPreparationPhase, GuiSeekPreparationRuntimeSnapshot,
    GuiSeekPreparationState, GuiShellAction, GuiShellModal, GuiShellView, GuiStreamHelperHealth,
    GuiStreamHelperRemediationRuntimeSnapshot, GuiStreamHelperRuntimeSnapshot,
    GuiTransientNotificationLevel, GuiWidgetEguiRenderer, GuiWidgetKind, GuiWidgetNode,
    MainWindowRuntimeSnapshot, SorotteGuiShellAppState,
};

use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use sorotte_media_match::{MediaMatchAutoplayPolicy, MediaMatchSettings};
use sorotte_plex::{
    PlexMediaType, PlexPlaylistUri, PlexServerConnectionKind, format_plex_playlist_uri,
};

mod configuration_views;
mod dialogs_public_servers_layout;
mod main_window_controls;
mod playlist_shell_status_renderer;
