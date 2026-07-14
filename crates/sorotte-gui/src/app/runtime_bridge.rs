#[cfg(test)]
mod tests;

use sorotte_client_app::app_boundary::{
    commands::LocalOffsetCommand, state::StoredClientSettingsMvp,
};
use sorotte_secret::SecretValue;

use super::render_io::GuiDroppedFilesRequest;
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::{
    GuiConfigStorageChangeTarget, GuiMediaSourceProviderId, GuiPendingOperationKind,
    GuiPluginSelection, GuiSavedConfigurationRuntimeSnapshot, GuiShellAction, GuiShellView,
    GuiTransientNotificationLevel, SorotteGuiShellAppState, shuffle_playlist_entries_in_place,
};
use super::support::format_offset_command;

mod pending;
mod preview_bridge;
mod request_preview;
mod requests;
mod traits;

pub(in crate::app) use pending::{
    GuiPendingCompletionRequest, GuiPendingRoomChangeRequest, GuiSharedPlaylistOpenDispatch,
    GuiSharedPlaylistOpenItem,
};
pub(in crate::app) use preview_bridge::GuiPreviewRuntimeBridge;
pub(in crate::app) use requests::{GuiPlexPlaylistJobCancellationReason, GuiRuntimeRequest};
#[cfg(test)]
pub(in crate::app) use traits::GuiPreviewRuntimeOwner;
pub(in crate::app) use traits::{
    GuiNativeRuntimeBridge, GuiNativeRuntimePump, GuiNoopRuntimePump, GuiQueuedRuntimeOwner,
};
