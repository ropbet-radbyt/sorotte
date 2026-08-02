use sorotte_client_app::app_boundary::commands::controlled_room_base_name_legacy_compatible;

use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::{
    FirstRunConfigurationDialogState, GuiConfigStorageChangeTarget, GuiConfigurationTab,
    GuiPendingOperationKind, GuiPlaylistDefaultSourceState, GuiPlaylistSourceState,
    GuiPlexPlaylistSearchResult, GuiPlexServerReachability, GuiPlexServerRow, GuiPluginSelection,
    GuiSeekPreparationDegradedReason, GuiSeekPreparationPhase, GuiSettingApplyRequirement,
    GuiSettingValueOrigin, GuiShellModal, GuiShellView, GuiStreamHelperHealth,
    GuiTransientNotificationLevel, SecretDraft, SettingId, SorotteGuiShellAppState,
    SynchronizationProfileId, detect_synchronization_profile, playlist_entries_from_multiline_text,
};
use super::support::{
    bool_label, configured_room_name_text, joined_room_name_text, normalized_editable_text,
    optional_seconds_text,
};
use super::widget_tree::{GuiLayoutMode, GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer};

mod configuration;
mod main_window;
mod media_search;
mod menus;
mod plugins;
mod public_servers;
mod shell;

#[cfg(test)]
mod tests;
