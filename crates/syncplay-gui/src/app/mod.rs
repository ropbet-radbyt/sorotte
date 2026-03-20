#![allow(dead_code)]

#[path = "../app_configuration_draft.rs"]
mod configuration_draft;
#[path = "../app_connection_workflows.rs"]
mod connection_workflows;
#[path = "../app_feedback_workflows.rs"]
mod feedback_workflows;
#[path = "../app_launcher.rs"]
mod launcher;
#[path = "../app_local_command_dispatch.rs"]
mod local_command_dispatch;
#[path = "../app_main_window_workflows.rs"]
mod main_window_workflows;
#[path = "../app_media_search_cache.rs"]
mod media_search_cache;
#[path = "../app_media_workflows.rs"]
mod media_workflows;
#[path = "../mpv_launch.rs"]
mod mpv_launch;
#[path = "../app_native_host.rs"]
mod native_host;
#[path = "../app_playlist_workflows.rs"]
mod playlist_workflows;
#[path = "../app_reducer.rs"]
mod reducer;
#[path = "../remote_services.rs"]
mod remote_services;
#[path = "../app_render_actions.rs"]
mod render_actions;
#[path = "../app_render_egui.rs"]
mod render_egui;
#[path = "../app_render_io.rs"]
mod render_io;
#[path = "../app_runtime_bridge.rs"]
mod runtime_bridge;
#[path = "../app_runtime_detached.rs"]
mod runtime_detached;
#[path = "../app_runtime_localization.rs"]
mod runtime_localization;
#[path = "../app_runtime_owner.rs"]
mod runtime_owner;
#[path = "../app_runtime_pump.rs"]
mod runtime_pump;
#[path = "../app_runtime_queue.rs"]
mod runtime_queue;
#[path = "../app_runtime_stack.rs"]
mod runtime_stack;
#[path = "../app_runtime_updates.rs"]
mod runtime_updates;
#[path = "../app_shell_core.rs"]
mod shell_core;
#[path = "../app_shell_projection.rs"]
mod shell_projection;
#[path = "../app_shell_state.rs"]
mod shell_state;
#[path = "../app_shell_workflows.rs"]
mod shell_workflows;
#[cfg(test)]
#[path = "../app_smoke.rs"]
mod smoke_tests;
#[path = "../app_startup.rs"]
mod startup;
#[path = "../app_startup_support.rs"]
mod startup_support;
#[path = "../app_state_integrity.rs"]
mod state_integrity;
#[path = "../app_support.rs"]
mod support;
#[cfg(test)]
mod testing;
#[path = "../app_ui_state.rs"]
mod ui_state;
#[path = "../app_widget_projection.rs"]
mod widget_projection;
#[path = "../app_widget_tree.rs"]
mod widget_tree;
#[path = "../app_widget_views.rs"]
mod widget_views;

use self::render_egui::GuiWidgetEguiRenderer;
use self::render_io::GuiDroppedFilesTarget;
#[allow(unused_imports)]
use self::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiNoopRuntimePump, GuiPendingCompletionRequest,
    GuiPendingRoomChangeRequest, GuiPreviewRuntimeBridge, GuiPreviewRuntimeOwner,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
#[allow(unused_imports)]
use self::runtime_owner::{
    GuiAttachedMediaSearchBuildProgress, GuiAttachedMediaSearchBuildStatus,
    GuiAttachedMediaSearchIndex, GuiAttachedMediaSearchRootIndex,
    GuiAttachedMediaSearchRootRefreshResult, GuiPendingAttachedMediaResolution,
    GuiPersistedConfigRuntimeOwner,
};
use self::runtime_queue::GuiQueuedRuntimeBridgeHandle;
#[allow(unused_imports)]
use self::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiLoopbackSessionTransportDriver, GuiOwnedPlayer,
    GuiPlayerLaunchRuntimeState, GuiQueuedSessionTransportHandle, GuiSessionRuntimeAdapter,
    GuiSessionTransportDriver, GuiTcpSessionTransportDriver, GuiTestPlayerAdapter,
};
#[allow(unused_imports)]
use self::shell_state::*;
use self::startup::run_gui_host_with_startup_actions_and_gui_state;
#[cfg(test)]
use self::ui_state::legacy_gui_qsettings_store_path;
use self::ui_state::{
    GuiPersistedUiState, load_gui_ui_state_from_root, persist_gui_ui_state_at_root,
};
use self::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use syncplay_client_app::app_boundary::{
    persistence::upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    state::StoredClientSettingsMvp,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiLaunchMode {
    FirstRun,
    ExistingConfig,
}

impl GuiLaunchMode {
    fn label(self) -> &'static str {
        match self {
            Self::FirstRun => "first-run",
            Self::ExistingConfig => "existing-config",
        }
    }
}

const LEGACY_GUI_QSETTINGS_STORE_NAMES: [&str; 5] = [
    "PlayerList",
    "MediaBrowseDialog",
    "MainWindow",
    "Interface",
    "MoreSettings",
];
const DEFAULT_MAIN_WINDOW_AUTOPLAY_THRESHOLD: usize = 2;

trait GuiAppHost {
    type Output;

    fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output;
}

#[path = "../live_python_interop.rs"]
pub(crate) mod live_python_interop;
#[path = "../semantic_driver.rs"]
pub(crate) mod semantic_driver;
#[path = "../semantic_smoke.rs"]
pub(crate) mod semantic_smoke;

pub fn run_syncplay_gui() {
    launcher::run_syncplay_gui();
}
