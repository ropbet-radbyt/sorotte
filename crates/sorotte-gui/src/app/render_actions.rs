use sorotte_client_app::app_boundary::{
    commands::{
        controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
    },
    language::SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY,
};

use super::mpv_launch;
use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::{
    GuiConfigurationTab, GuiDialogControlKind, GuiDraftRuntimeSnapshot, GuiMediaSourceProviderId,
    GuiPlaylistDefaultSourceId, GuiPluginSelection, GuiShellAction, GuiShellModal, GuiShellView,
    GuiTransientNotificationLevel, SorotteGuiShellAppState, browser_domain_from_url,
    load_playlist_entries_from_path, playlist_entries_from_multiline_text,
    save_playlist_entries_to_path,
};
use super::support::{nonempty_room_name_text, normalized_editable_text};
use super::widget_tree::GuiWidgetNode;

mod buttons;
mod helpers;
mod inputs;
mod lists;
mod menu;
mod surface;
