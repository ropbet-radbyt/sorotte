use super::GuiRuntimeState;
use crate::app::playlist_model::{self, GuiPlaylistSources};
use crate::app::shell_state::*;

impl GuiRuntimeState {
    fn playlist_sources(&self) -> GuiPlaylistSources<'_> {
        GuiPlaylistSources {
            default_source: self
                .playlist
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match.model,
            plex: &self.plex.model,
            plugin_enablement: self.settings.plugin_enablement,
        }
    }
    pub(in crate::app) fn current_shared_playlist_entries(&self) -> Vec<String> {
        playlist_model::current_shared_playlist_entries(&self.playlist.main_window.playlist)
    }
    pub(in crate::app) fn normalize_shared_playlist_entries(entries: Vec<String>) -> Vec<String> {
        playlist_model::normalize_shared_playlist_entries(entries)
    }
    pub(in crate::app) fn playlist_source_state_for_entry(
        &self,
        entry: &str,
    ) -> GuiPlaylistSourceState {
        self.playlist_sources()
            .playlist_source_state_for_entry(entry)
    }
    pub(in crate::app) fn set_playlist_source_state(
        &mut self,
        index: usize,
        source_state: GuiPlaylistSourceState,
    ) -> bool {
        let Some(row) = self.playlist.main_window.playlist.get(index) else {
            return false;
        };
        let mut source_state = self
            .playlist_sources()
            .refreshed_playlist_source_state_for_entry(&row.label, source_state);
        let row = &mut self.playlist.main_window.playlist[index];
        source_state.entry_id = row.entry_id;
        row.source_state = source_state;
        true
    }
    pub(in crate::app) fn refresh_playlist_source_states(&mut self) {
        let states = self
            .playlist
            .main_window
            .playlist
            .iter()
            .map(|row| {
                self.playlist_sources()
                    .refreshed_playlist_source_state_for_entry(&row.label, row.source_state.clone())
            })
            .collect::<Vec<_>>();
        for (row, state) in self.playlist.main_window.playlist.iter_mut().zip(states) {
            row.source_state = state;
        }
        self.playlist.main_window.playlist_default_source = self
            .playlist_sources()
            .refreshed_playlist_source_default_state(
                self.playlist.main_window.playlist_default_source.clone(),
            );
    }
    pub(in crate::app) fn remember_shared_playlist_undo_snapshot(&mut self) {
        self.playlist.source_undo_snapshot = Some(
            self.playlist
                .main_window
                .playlist
                .iter()
                .map(|row| row.source_state.clone())
                .collect(),
        );
        self.playlist.entry_id_undo_snapshot = Some(
            self.playlist
                .main_window
                .playlist
                .iter()
                .map(|row| row.entry_id)
                .collect(),
        );
        self.playlist.undo_snapshot = Some(self.current_shared_playlist_entries());
    }
    pub(in crate::app) fn remember_shared_playlist_undo_snapshot_if_changed(
        &mut self,
        next: &[String],
    ) {
        if self.current_shared_playlist_entries() != next {
            self.remember_shared_playlist_undo_snapshot();
        }
    }
    pub(in crate::app) fn apply_shared_playlist_entries(
        &mut self,
        entries: Vec<String>,
        selected: Option<usize>,
        local: bool,
    ) {
        let sources = GuiPlaylistSources {
            default_source: self
                .playlist
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match.model,
            plex: &self.plex.model,
            plugin_enablement: self.settings.plugin_enablement,
        };
        playlist_model::apply_shared_playlist_entries(
            &mut self.playlist.main_window,
            &mut self.playlist.selection,
            &mut self.playlist.selection_is_local,
            &sources,
            entries,
            selected,
            local,
        );
    }
    pub(in crate::app) fn next_shared_playlist_shuffle_seed(
        &mut self,
        entries: &[String],
        index: usize,
        remaining: bool,
    ) -> u64 {
        playlist_model::next_shared_playlist_shuffle_seed(
            &mut self.playlist.shuffle_nonce,
            entries,
            index,
            remaining,
        )
    }
    pub(in crate::app) fn shared_playlist_entries_after_media_open_from_state_with_current_index(
        &self,
        opened: Vec<String>,
        insert_slot: Option<usize>,
        current_index: Option<usize>,
    ) -> (Vec<String>, Option<usize>) {
        playlist_model::shared_playlist_entries_after_media_open(
            &self.current_shared_playlist_entries(),
            current_index,
            opened,
            insert_slot,
        )
    }
}

impl GuiRuntimeState {
    pub(in crate::app) fn apply_main_window_runtime_snapshot(
        &mut self,
        snapshot: MainWindowRuntimeSnapshot,
    ) -> bool {
        let sources = crate::app::playlist_model::GuiPlaylistSources {
            default_source: self
                .playlist
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match.model,
            plex: &self.plex.model,
            plugin_enablement: self.settings.plugin_enablement,
        };
        let result = crate::app::main_window_projection::GuiMainWindowProjection {
            main_window: &mut self.playlist.main_window,
            selection: &mut self.playlist.selection,
            main_window_playlist_selection_is_local: &mut self.playlist.selection_is_local,
            playlist_undo_snapshot: &mut self.playlist.undo_snapshot,
            playlist_source_undo_snapshot: &mut self.playlist.source_undo_snapshot,
            playlist_entry_id_undo_snapshot: &mut self.playlist.entry_id_undo_snapshot,
            pending_local_ready_target: &mut self.session.pending_local_ready_target,
            menus: &mut self.session.menus,
            sources,
        }
        .apply(snapshot);
        if let Err(message) = result {
            return self.record_action_error(message);
        }
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }
}

#[cfg(test)]
impl GuiRuntimeState {
    pub(in crate::app) fn playlist_edit_model(
        &mut self,
    ) -> crate::app::playlist_model::GuiPlaylistEditing<'_> {
        let sources = crate::app::playlist_model::GuiPlaylistSources {
            default_source: self
                .playlist
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match.model,
            plex: &self.plex.model,
            plugin_enablement: self.settings.plugin_enablement,
        };
        crate::app::playlist_model::GuiPlaylistEditing {
            main_window: &mut self.playlist.main_window,
            selection: &mut self.playlist.selection,
            playlist_undo_snapshot: &mut self.playlist.undo_snapshot,
            playlist_source_undo_snapshot: &mut self.playlist.source_undo_snapshot,
            playlist_entry_id_undo_snapshot: &mut self.playlist.entry_id_undo_snapshot,
            selection_is_local: &mut self.playlist.selection_is_local,
            shuffle_nonce: &mut self.playlist.shuffle_nonce,
            sources,
        }
    }
}
