mod child_process;
mod configuration_draft;
mod connection_workflows;
mod feature_slices;
mod feedback_workflows;
mod launcher;
mod local_command_dispatch;
mod main_window_workflows;
mod media_match_support;
mod media_search_cache;
mod media_workflows;
mod mpv_launch;
mod native_host;
mod playlist_workflows;
mod reducer;
mod remote_services;
mod render_actions;
mod render_egui;
mod render_io;
mod runtime_bridge;
mod runtime_detached;
mod runtime_localization;
mod runtime_owner;
mod runtime_pump;
mod runtime_queue;
mod runtime_stack;
mod runtime_updates;
mod shell_core;
mod shell_projection;
mod shell_state;
mod shell_workflows;
#[cfg(test)]
mod smoke_tests;
mod startup;
mod startup_support;
mod state_integrity;
mod stream_support;
mod support;
#[cfg(test)]
mod testing;
mod ui_state;
mod widget_projection;
mod widget_tree;
mod widget_views;
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::render_egui::GuiWidgetEguiRenderer;
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::render_io::GuiDroppedFilesTarget;
#[cfg(feature = "gui-semantic-smoke")]
use self::runtime_bridge::GuiNativeRuntimeBridge;
#[cfg(test)]
use self::runtime_bridge::GuiPendingRoomChangeRequest;
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::runtime_bridge::{
    GuiPendingCompletionRequest, GuiPreviewRuntimeBridge, GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use self::runtime_owner::GuiPersistedConfigRuntimeOwner;
#[cfg(test)]
use self::runtime_owner::{
    GuiAttachedMediaSearchBuildProgress, GuiAttachedMediaSearchBuildState,
    GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
    GuiAttachedMediaSearchRootIndex, GuiAttachedMediaSearchRootRefreshResult,
    GuiPendingAttachedMediaResolution,
};
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::runtime_queue::GuiQueuedRuntimeBridgeHandle;
#[cfg(test)]
use self::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiPlayerLaunchRuntimeState, GuiSessionRuntimeAdapter,
};
#[cfg(test)]
use self::runtime_stack::{GuiOwnedPlayer, GuiTestPlayerAdapter};
use self::shell_state::*;
#[cfg(feature = "gui-semantic-smoke")]
use self::startup::run_gui_host_with_startup_actions_and_gui_state;
#[cfg(test)]
use self::ui_state::legacy_gui_qsettings_store_path;
#[cfg(feature = "gui-semantic-smoke")]
use self::ui_state::load_gui_ui_state_from_root;
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::ui_state::{GuiPersistedUiState, persist_gui_ui_state_at_root};
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use self::widget_tree::{GuiWidgetKind, GuiWidgetNode};
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use sorotte_client_app::app_boundary::persistence::upsert_sorotte_ini_stored_client_settings_mvp_at_path;
#[cfg(any(test, feature = "gui-semantic-smoke"))]
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiLaunchMode {
    FirstRun,
    ExistingConfig,
}

impl GuiLaunchMode {
    #[cfg(test)]
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

    fn render(&mut self, state: SorotteGuiShellAppState) -> Self::Output;
}

#[cfg(any(
    all(test, feature = "live-python-interop"),
    all(feature = "gui-semantic-smoke", feature = "live-python-interop")
))]
pub(crate) mod live_python_interop;
#[cfg(feature = "gui-semantic-smoke")]
pub(crate) mod semantic_driver;
#[cfg(feature = "gui-semantic-smoke")]
pub(crate) mod semantic_smoke;

pub fn run_sorotte_gui() {
    launcher::run_sorotte_gui();
}
