use super::{
    GuiAppHost, GuiPersistedUiState, GuiShellAction, GuiTextPreviewHost, SyncplayGuiShellAppState,
    run_gui_host, shell_widget_preview, startup_notice, startup_preview,
};

use crate::app::GuiShellView;
use crate::app::testing::support::{
    TEST_USERNAME, test_default_syncplay_config_env_root, test_default_syncplay_config_target,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

mod bootstrap_settings;
mod host_and_state_overlay;
mod notices_and_remote_actions;
mod startup_action_sources;
