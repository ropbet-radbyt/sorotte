use super::{GuiClientCoreChatSessionRuntimeAdapter, GuiSessionRuntimeAdapter};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiInteractionRuntimeSnapshot, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SyncplayGuiShellAppState,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use syncplay_client_core::ReconnectTransitionNotification;

mod chat_projection_tests;
mod controller_autoplay_tests;
mod playlist_tests;
mod public_server_tests;
mod session_config_tests;
mod session_transition_tests;
