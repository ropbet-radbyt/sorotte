#![allow(unused_imports)]

use syncplay_client_app::app_boundary::commands::controlled_room_base_name_legacy_compatible;

use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::{
    GuiPendingOperationKind, GuiShellModal, GuiShellView, SyncplayGuiShellAppState,
    browser_domain_from_url, browser_is_url, browser_uri_is_trusted,
};
use super::support::{bool_label, normalized_editable_text, optional_seconds_text};
use super::widget_tree::{GuiLayoutMode, GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer};

#[path = "app_widget_views/configuration.rs"]
mod configuration;
#[path = "app_widget_views/main_window.rs"]
mod main_window;
#[path = "app_widget_views/media_search.rs"]
mod media_search;
#[path = "app_widget_views/menus.rs"]
mod menus;
#[path = "app_widget_views/public_servers.rs"]
mod public_servers;
#[path = "app_widget_views/shell.rs"]
mod shell;

#[cfg(test)]
#[path = "app_widget_views/tests.rs"]
mod tests;
