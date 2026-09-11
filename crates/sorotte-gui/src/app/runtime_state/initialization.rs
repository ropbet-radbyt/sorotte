use super::GuiRuntimeState;
use crate::app::feature_slices::{
    media_match, media_resolution, player, playlist, plex, session, settings, updates,
};
use crate::app::shell_state::*;
use crate::app::ui_state::GuiUpdateCheckState;
use sorotte_client_app::app_boundary::state::{
    StoredClientSettings, stored_client_settings_runtime_snapshot,
};

impl GuiRuntimeState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettings) -> Self {
        let runtime_settings = stored_client_settings_runtime_snapshot(settings);
        let mut shell_settings = settings.clone();
        shell_settings.room = runtime_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str().to_owned())
            .map(|room| {
                runtime_settings
                    .config
                    .connection
                    .controlled_room_password
                    .as_ref()
                    .map_or(room.clone(), |password| {
                        format!("{room}:{}", password.expose_secret())
                    })
            });
        let mut state = Self {
session: session::RuntimeView {
commands: GuiCommandAvailabilityState::default(),
command_overrides: GuiCommandAvailabilityRuntimeOverride::default(),
menu_overrides: Vec::new(),
menus: MenuDialogShellState::from_stored_settings(&shell_settings),
pending_operation: None,
pending_local_ready_target: None,
pending_saved_server_connect_intent: None,
outgoing_chat_message: None,
public_servers: PublicServerBrowserShellState::from_stored_settings(&shell_settings),
},
player: player::RuntimeView {
setup_issue: None,
seek_preparation: None,
seek_preparation_degraded_reason: None,
stream_helper: Default::default(),
stream_helper_remediation: Default::default(),
},
playlist: playlist::RuntimeView {
main_window: MainWindowShellState::from_stored_settings(&shell_settings),
selection: GuiSelectionState::default(),
selection_is_local: false,
undo_snapshot: None,
source_undo_snapshot: None,
entry_id_undo_snapshot: None,
shuffle_nonce: 0,
},
media_resolution: media_resolution::RuntimeView {
index_status: Default::default(),
search: MediaSearchWorkflowShellState::from_stored_settings(&shell_settings),
last_dialog_directory: None,
},
media_match: media_match::RuntimeView {
model: GuiMediaMatchState::from_stored_settings(&shell_settings),
remediation: Default::default(),
},
plex: plex::RuntimeView {
model: GuiPlexState::from_stored_settings(&shell_settings),
playlist_search: None,
},
settings: settings::RuntimeView {
active_application_language: shell_settings.language.clone(),
active_application_force_gui_prompt: shell_settings.force_gui_prompt,
plugin_enablement: GuiPluginEnablementState::from_stored_settings(&shell_settings),
config_storage: GuiConfigStorageRuntimeSnapshot::default(),
pending_storage_target: None,
saved: shell_settings.clone(),
draft: FirstRunConfigurationDialogDraft::from_stored_settings(&shell_settings),
validation: GuiValidationState::default(),
runtime_validation_issues: Vec::new(),
},
updates: updates::RuntimeView {
model: GuiUpdateCheckState::default(),
policy: updates::RuntimePolicy {
 automatic: shell_settings.check_for_updates_automatically == Some(true),
 last_checked_for_updates: shell_settings.last_checked_for_updates.clone(),
 language: crate::app::runtime_localization::normalized_runtime_language_tag_or_default(shell_settings.language.as_deref()).to_owned(),
 channel: shell_settings.update_channel.as_deref().and_then(crate::app::support::normalized_editable_text).map(|value| value.to_ascii_lowercase()),
},
},
};
        state.refresh_playlist_source_states();
        state.default_selection_from_surfaces();
        state.apply_selection_to_surfaces();
        state.refresh_validation();
        state
    }
}
impl GuiRuntimeState {
    pub(super) fn default_selection_from_surfaces(&mut self) {
        crate::app::selection_projection::default_selection_from_surfaces(
            &self.playlist.main_window,
            &self.media_resolution.search,
            &self.session.menus,
            &mut self.playlist.selection,
            &mut self.playlist.selection_is_local,
        );
    }
    pub(super) fn normalize_selection(&mut self) {
        crate::app::selection_projection::normalize_selection(
            &self.playlist.main_window,
            &self.media_resolution.search,
            &self.session.menus,
            &mut self.playlist.selection,
            &mut self.playlist.selection_is_local,
        );
    }
}
