use super::*;
use crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome;
use crate::app::runtime_owner::{GuiPendingStreamLoadContext, GuiPlaylistResolutionCoordinator};
use crate::app::{GuiPlaylistSourceState, GuiStreamHelperHealth, GuiStreamHelperRuntimeSnapshot};
use sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible;

fn write_persisted_media_search_root_index(
    gui_root: &std::path::Path,
    media_root: &std::path::Path,
    built_at_unix_ms: u64,
    candidates_by_name: &[(&str, &[&str])],
) {
    let persisted = crate::app::media_search_cache::PersistedMediaSearchRootIndexV2 {
        version: 2,
        root_key: crate::app::media_search_cache::normalized_media_search_root_key(media_root),
        root_path: media_root.to_string_lossy().into_owned(),
        built_at_unix_ms,
        candidates_by_name: candidates_by_name
            .iter()
            .map(|(name, candidates)| {
                (
                    (*name).to_owned(),
                    candidates
                        .iter()
                        .map(|candidate| (*candidate).to_owned())
                        .collect(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>(),
    };
    crate::app::media_search_cache::persist_media_search_root_index_at_root(gui_root, &persisted)
        .expect("persisted media-search cache fixture should be written");
}

fn without_media_match_runtime_snapshots(actions: Vec<GuiShellAction>) -> Vec<GuiShellAction> {
    actions
        .into_iter()
        .filter(|action| !matches!(action, GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(_)))
        .collect()
}

fn expected_playlist_source_states_for_entries(
    state: &SorotteGuiShellAppState,
    entries: &[&str],
    detail: Option<&str>,
) -> Vec<GuiPlaylistSourceState> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut source_state = state
                .main_window
                .playlist
                .get(index)
                .filter(|row| row.label == *entry)
                .map(|row| row.source_state.clone())
                .unwrap_or_else(|| state.playlist_source_state_for_entry(entry));
            if let Some(detail) = detail {
                source_state.detail = Some(detail.to_owned());
            }
            source_state
        })
        .collect()
}

mod attached_media_open_seek;
mod attached_state_sync;
mod desync_slowdown;
mod media_search_cache;
mod offsets_and_recent_rewind;
mod playlist_index_and_search_seed;
mod playlist_reset_and_desync_seek;
mod playlist_switch_and_offsets;
mod readiness_and_unpause;
mod room_playstate_matching;
mod unresolved_playlist_retry;
