use super::{GuiLayoutMode, GuiWidgetRenderer};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiConfigurationTab, GuiDraftRuntimeSnapshot, GuiErrorRuntimeSnapshot,
    GuiMediaIndexRuntimeSnapshot, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiPlexRuntimeSnapshot, GuiPlexServerReachability,
    GuiPlexServerRow, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiStreamHelperHealth, GuiStreamHelperRemediationRuntimeSnapshot,
    GuiStreamHelperRuntimeSnapshot, GuiTransientNotificationLevel, GuiWidgetEguiRenderer,
    GuiWidgetKind, GuiWidgetNode, MainWindowRuntimeSnapshot, SyncplayGuiShellAppState,
};

use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use syncplay_plex::PlexServerConnectionKind;

mod configuration_views;
mod dialogs_public_servers_layout;
mod main_window_controls;
mod playlist_shell_status_renderer;
