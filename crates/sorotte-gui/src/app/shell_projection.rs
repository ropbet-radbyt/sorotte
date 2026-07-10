mod app_state;
mod basic_state;
mod main_window;
mod media_search;
mod menu_dialog;
mod public_servers;

use sorotte_client_app::app_boundary::state::{
    ClientConfig, StoredClientSettingsMvp,
    stored_client_settings_runtime_snapshot_legacy_compatible,
};

use super::shell_state::{
    FirstRunConfigurationDialogDraft, GuiCommandAvailabilityRuntimeOverride,
    GuiCommandAvailabilityState, GuiControlledRoomCreateSessionState,
    GuiControllerAuthEditSessionState, GuiFocusedConfigurationControlState,
    GuiMainWindowUserEditSessionState, GuiMediaIndexStatusState, GuiPlaylistDefaultSourceState,
    GuiPlaylistTextEditSessionState, GuiPublicServerEditSessionState,
    GuiRoomHistoryEditSessionState, GuiSelectionState, GuiTextEditSessionState,
    GuiTransientNotification, GuiUrlEditSessionState, GuiValidationState, MainWindowChatRow,
    MainWindowPlaybackControls, MainWindowPlaylistRow, MainWindowRoomRow, MainWindowShellState,
    MainWindowUserRow, MediaSearchDirectoryRow, MediaSearchWorkflowRuntimeFlags,
    MediaSearchWorkflowShellState, MenuActionShellItem, MenuDialogShellState,
    MenuSectionShellState, PublicServerBrowserRow, PublicServerBrowserRuntimeFlags,
    PublicServerBrowserShellState, SorotteGuiShellAppState,
};
#[cfg(test)]
use super::shell_state::{GuiPendingOperationState, GuiShellModal};
use super::support::autoplay_threshold_from_settings;
#[cfg(test)]
use super::support::{bool_label, optional_index_text, optional_seconds_text};
