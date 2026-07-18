use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use eframe::egui;
use sorotte_client_app::app_boundary::commands::{
    LocalInputCommand, LocalOffsetCommand, parse_local_input_command,
};

use super::GuiAppHost;
use super::local_command_dispatch::GuiShellDispatchPlan;
use super::render_egui::{GuiPlaybackPromptKind, GuiWidgetEguiRenderer};
use super::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use super::runtime_bridge::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiNoopRuntimePump, GuiPendingRoomChangeRequest,
    GuiPreviewRuntimeBridge, GuiQueuedRuntimeOwner,
};
use super::runtime_owner::{GuiCorePlayerConfigurationHealth, GuiPersistedConfigRuntimeOwner};
use super::runtime_queue::{
    GuiQueuedRuntimeBridge, GuiQueuedRuntimeBridgeHandle, GuiRuntimeThreadUnavailablePump,
    GuiThreadedRuntimeOwnerPump,
};
use super::shell_state::{
    GuiShellAction, GuiTransientNotificationLevel, MenuActionId, SorotteGuiShellAppState,
};
use super::startup::sorotte_gui_qsettings_root_from_env;
use super::startup_support::env_trimmed;
use super::support::{nonempty_room_name_text, normalized_editable_text};
use super::ui_state::{GuiPersistedUiState, persist_gui_ui_state_at_root};
#[cfg(test)]
use super::widget_tree::GuiWidgetTextPreviewRenderer;

mod app_core;
mod eframe_app;
mod eframe_host;
#[cfg(test)]
mod preview_host;

#[cfg(test)]
mod tests;

pub(in crate::app) struct GuiNativeApp {
    state: SorotteGuiShellAppState,
    runtime: Box<dyn GuiNativeRuntimeBridge>,
    runtime_pump: Box<dyn GuiNativeRuntimePump>,
    runtime_repaint_handle: Option<GuiQueuedRuntimeBridgeHandle>,
    gui_state_root: Option<PathBuf>,
    test_drop_request: Option<GuiDroppedFilesRequest>,
    playback_prompt: Option<GuiPlaybackPromptKind>,
    playback_prompt_buffer: String,
    playback_prompt_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiNativeShellEffect {
    PickMediaFiles,
    CloseWindow,
    OpenPlaybackPrompt(GuiPlaybackPromptKind),
    RequestUndoSeek,
    OpenHelp,
}

pub(in crate::app) struct GuiEframeNativeHost {
    runtime: Option<Box<dyn GuiNativeRuntimeBridge>>,
    runtime_pump: Option<Box<dyn GuiNativeRuntimePump>>,
    runtime_repaint_handle: Option<GuiQueuedRuntimeBridgeHandle>,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(in crate::app) struct GuiTextPreviewHost;
