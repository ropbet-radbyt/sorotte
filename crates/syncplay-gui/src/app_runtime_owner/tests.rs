use std::path::PathBuf;

use super::GuiPersistedConfigRuntimeOwner;

use crate::app::testing::support::{
    browser_runtime_rooms, browser_runtime_user, pump_and_apply_runtime_owner_actions,
    pump_and_apply_runtime_owner_actions_until, test_temp_root,
};
use crate::app::{
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiInteractionRuntimeSnapshot,
    GuiLaunchMode, GuiOwnedPlayer, GuiPendingCompletionRequest, GuiPendingOperationKind,
    GuiPersistedUiState, GuiPlayerLaunchRuntimeState, GuiQueuedRuntimeBridgeHandle,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiSessionRuntimeAdapter, GuiShellAction,
    GuiShellView, GuiTestPlayerAdapter, GuiTransientNotificationLevel,
    MainWindowRuntimeChatSnapshot, MainWindowRuntimeSnapshot, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SyncplayGuiRuntimeSnapshot, SyncplayGuiShellAppState,
    legacy_gui_qsettings_store_path, persist_gui_ui_state_at_root,
};
use syncplay_client_app::app_boundary::persistence::{
    load_syncplay_ini_stored_client_settings_mvp_from_path,
    upsert_syncplay_ini_stored_client_settings_mvp_at_path,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use syncplay_player_api::PlayerAdapter;

#[path = "tests/connection_runtime_tests.rs"]
mod connection_runtime_tests;
#[path = "tests/persistence_tests.rs"]
mod persistence_tests;
#[path = "tests/player_runtime_tests.rs"]
mod player_runtime_tests;
#[path = "tests/playlist_runtime_tests.rs"]
mod playlist_runtime_tests;
#[path = "tests/session_runtime_tests.rs"]
mod session_runtime_tests;
#[path = "tests/startup_tests.rs"]
mod startup_tests;
#[path = "tests/transport_tests.rs"]
mod transport_tests;
