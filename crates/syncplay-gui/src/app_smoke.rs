#![allow(unused_imports)]

use std::path::PathBuf;

use super::testing::support::{
    pump_and_apply_runtime_owner_actions, pump_and_apply_runtime_owner_actions_until,
};
use syncplay_client_app::app_boundary::{
    persistence::load_syncplay_ini_stored_client_settings_mvp_from_path,
    state::{AutoplayThresholdOverride, StoredClientSettingsMvp},
};
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

use super::runtime_bridge::{
    GuiPendingCompletionRequest, GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::{
    GuiPendingOperationKind, GuiShellAction, GuiShellView, SyncplayGuiShellAppState,
};
use super::{
    GuiPreviewRuntimeBridge, live_python_interop,
    upsert_syncplay_ini_stored_client_settings_mvp_at_path,
};

#[path = "app_smoke/live_python_smoke.rs"]
mod live_python_smoke;
#[path = "app_smoke/managed_mpv_smoke.rs"]
mod managed_mpv_smoke;
#[path = "app_smoke/portable_persistence_transport_smoke.rs"]
mod portable_persistence_transport_smoke;
#[path = "app_smoke/portable_script_parity_smoke.rs"]
mod portable_script_parity_smoke;
#[path = "app_smoke/portable_tcp_reconnect_smoke.rs"]
mod portable_tcp_reconnect_smoke;
