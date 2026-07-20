use super::{
    GuiAttachedPlayerRuntimeAction, GuiClientCoreChatSessionRuntimeAdapter,
    GuiSessionRuntimeAdapter,
};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiInteractionRuntimeSnapshot, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot, MenuActionId,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, SorotteGuiShellAppState,
};
use sorotte_client_app::app_boundary::state::{
    StoredClientSettingsMvp, stored_client_settings_runtime_snapshot_legacy_compatible,
};
use sorotte_client_core::{
    ConnectionPhase, CoordinatorPlayerCommand, LogicalMediaId, MediaLoadIntent, MediaTransportKind,
    ReconnectTransitionNotification,
};
use sorotte_player_api::{
    PlayerMediaGeneration, PlayerObservationTimestamp, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};

fn sync_adapter_to_saved_session_settings(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
    state: &SorotteGuiShellAppState,
) {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&state.saved_configuration);
    GuiSessionRuntimeAdapter::sync_runtime_settings(adapter, &runtime_settings)
        .expect("saved settings should initialize the active test session");
}

mod chat_projection_tests;
mod controller_autoplay_tests;
mod playback_barrier_integration_tests;
mod playlist_tests;
mod public_server_tests;
mod session_config_tests;
mod session_transition_tests;
