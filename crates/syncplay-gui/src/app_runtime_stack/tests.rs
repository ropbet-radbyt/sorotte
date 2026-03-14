use super::{GuiClientCoreChatSessionRuntimeAdapter, GuiSessionRuntimeAdapter};

use crate::app::testing::support::browser_runtime_user;
use crate::app::{
    GuiInteractionRuntimeSnapshot, GuiShellAction, GuiTransientNotificationLevel,
    MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SyncplayGuiShellAppState,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use syncplay_client_core::ReconnectTransitionNotification;

#[path = "tests/chat_projection_tests.rs"]
mod chat_projection_tests;
#[path = "tests/controller_autoplay_tests.rs"]
mod controller_autoplay_tests;
#[path = "tests/playlist_tests.rs"]
mod playlist_tests;
#[path = "tests/public_server_tests.rs"]
mod public_server_tests;
#[path = "tests/session_transition_tests.rs"]
mod session_transition_tests;
