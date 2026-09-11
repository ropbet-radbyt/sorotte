use sorotte_client_app::app_boundary::{
    commands::{controlled_room_base_name, generate_room_password},
    language::SUPPORTED_RUNTIME_LANGUAGE_TAGS_DISPLAY,
};

use super::mpv_launch;
use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::{
    GuiConfigurationTab, GuiDialogControlKind, GuiDraftRuntimeSnapshot, GuiMediaSourceProviderId,
    GuiPlaylistDefaultSourceId, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiTransientNotificationLevel, MenuActionId, SecretDraft, SettingId, SorotteGuiShellAppState,
    browser_domain_from_url, playlist_entries_from_multiline_text, save_playlist_entries_to_path,
};
use super::support::{nonempty_room_name_text, normalized_editable_text};
use super::widget_tree::GuiWidgetNode;

mod buttons;
mod helpers;
mod inputs;
mod lists;
mod surface;
