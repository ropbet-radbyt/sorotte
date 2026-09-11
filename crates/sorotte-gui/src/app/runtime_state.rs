//! Mutable worker state, built directly from the eight feature inputs.
//! Worker actions are applied before the next queued command; UI-only state
//! stays in the shell.

use super::feature_slices::{
    media_match, media_resolution, player, playlist, plex, session, settings, updates,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GuiRuntimeState {
    pub(super) session: session::RuntimeView,
    pub(super) player: player::RuntimeView,
    pub(super) playlist: playlist::RuntimeView,
    pub(super) media_resolution: media_resolution::RuntimeView,
    pub(super) media_match: media_match::RuntimeView,
    pub(super) plex: plex::RuntimeView,
    pub(super) settings: settings::RuntimeView,
    pub(super) updates: updates::RuntimeView,
}

#[path = "runtime_state/playlist.rs"]
mod playlist_operations;

#[path = "runtime_state/settings.rs"]
mod settings_operations;

#[path = "runtime_state/initialization.rs"]
mod initialization;

#[path = "runtime_state/configuration_rebase.rs"]
mod configuration_rebase;

mod actions;

mod service_outputs;
