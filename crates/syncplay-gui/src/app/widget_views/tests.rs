use super::{GuiLayoutMode, GuiWidgetRenderer};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiConfigurationTab, GuiDraftRuntimeSnapshot, GuiMediaIndexRuntimeSnapshot,
    GuiPlayerSetupIssue, GuiPlayerSetupIssueKind, GuiPlayerSetupRuntimeSnapshot, GuiShellAction,
    GuiShellModal, GuiShellView, GuiStreamHelperHealth, GuiStreamHelperRemediationRuntimeSnapshot,
    GuiStreamHelperRuntimeSnapshot, GuiTransientNotificationLevel, GuiWidgetKind, GuiWidgetNode,
    MainWindowRuntimeSnapshot, SyncplayGuiShellAppState,
};

use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

mod configuration_views;
mod dialogs_public_servers_layout;
mod main_window_controls;
mod playlist_shell_status_renderer;
