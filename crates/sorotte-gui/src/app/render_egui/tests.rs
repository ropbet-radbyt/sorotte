use std::path::PathBuf;

use eframe::egui;
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use sorotte_plex::PlexMediaType;

use super::GuiWidgetEguiRenderer;
use crate::app::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use crate::app::shell_state::{
    GuiConfigStorageRuntimeSnapshot, GuiConfigurationTab, GuiDraftRuntimeSnapshot,
    GuiMediaSourceProviderId, GuiPendingOperationKind, GuiPlaylistDefaultSourceId,
    GuiPlexPlaylistSearchResult, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiStreamHelperHealth, GuiStreamHelperRemediationRuntimeSnapshot,
    GuiStreamHelperRuntimeSnapshot, MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot,
    MainWindowRuntimeUserSnapshot, SorotteGuiShellAppState,
};
use crate::app::testing::support::{TEST_USERNAME, browser_runtime_user};
use crate::app::widget_tree::{GuiWidgetKind, GuiWidgetNode};

mod action_mapping_surface;
mod dialogs_and_file_pickers;
mod drop_targets_and_playlist_labels;
mod edit_actions;
mod playlist_interactions;
mod playlist_source_mapping;
mod renderer_contract_and_layout;
mod workflow_and_stream_support;
