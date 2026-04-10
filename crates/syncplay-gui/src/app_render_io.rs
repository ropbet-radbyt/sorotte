use eframe::egui;
use rfd::FileDialog;

use super::render_egui::GuiWidgetEguiRenderer;
use super::shell_state::SyncplayGuiShellAppState;
use super::startup_support::env_trimmed;
use super::support::{normalized_editable_text, shared_playlist_entry_for_media_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiDroppedFilesTarget {
    Window,
    Playlist,
}

impl GuiDroppedFilesTarget {
    pub(super) fn parse(token: &str) -> Result<Self, String> {
        match token.trim() {
            "window" => Ok(Self::Window),
            "playlist" => Ok(Self::Playlist),
            other => Err(format!(
                "unknown dropped-files target {other:?}; expected 'window' or 'playlist'"
            )),
        }
    }

    pub(super) fn load_into_shared_playlist(self, state: &SyncplayGuiShellAppState) -> bool {
        matches!(self, Self::Playlist) || state.playlist_backed_media_opens_preferred()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiDroppedFilesRequest {
    pub(super) target: GuiDroppedFilesTarget,
    pub(super) paths: Vec<String>,
    pub(super) playlist_insert_slot: Option<usize>,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn shared_playlist_entries_for_media_paths(paths: Vec<String>) -> Vec<String> {
        paths
            .iter()
            .filter_map(|path| shared_playlist_entry_for_media_path(path))
            .collect()
    }

    pub(super) fn dropped_files_request_for_input(
        state: &SyncplayGuiShellAppState,
        playlist_drop_target_hovered: bool,
        playlist_drop_target_rect: Option<egui::Rect>,
        playlist_drop_target_slot: Option<usize>,
        pointer_hover_pos: Option<egui::Pos2>,
        dropped_files: Vec<egui::DroppedFile>,
    ) -> Option<GuiDroppedFilesRequest> {
        let paths = dropped_files
            .iter()
            .filter_map(Self::dropped_file_path)
            .collect::<Vec<_>>();
        let playlist_append_slot = Some(state.main_window.playlist.len());
        if paths.is_empty() {
            return None;
        }
        if state.playlist_backed_media_opens_preferred() {
            return Some(GuiDroppedFilesRequest {
                target: GuiDroppedFilesTarget::Playlist,
                paths,
                playlist_insert_slot: playlist_drop_target_slot.or(playlist_append_slot),
            });
        }
        let hovered_playlist_insert_slot = hovered_playlist_insert_slot(
            playlist_drop_target_hovered,
            playlist_drop_target_rect,
            pointer_hover_pos,
            playlist_drop_target_slot,
        );
        let hovered_playlist_target = hovered_playlist_insert_slot.is_some()
            || playlist_drop_target_hovered
            || playlist_drop_target_rect
                .zip(pointer_hover_pos)
                .is_some_and(|(rect, pointer)| rect.contains(pointer));
        let target = if hovered_playlist_target
            && GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(state)
        {
            GuiDroppedFilesTarget::Playlist
        } else {
            GuiDroppedFilesTarget::Window
        };
        Some(GuiDroppedFilesRequest {
            target,
            paths,
            playlist_insert_slot: matches!(target, GuiDroppedFilesTarget::Playlist)
                .then_some(hovered_playlist_insert_slot.or(playlist_append_slot))
                .flatten(),
        })
    }

    fn dropped_file_path(file: &egui::DroppedFile) -> Option<String> {
        if let Some(path) = file.path.as_ref() {
            return Some(path.to_string_lossy().into_owned());
        }
        normalized_editable_text(&file.name)
    }

    pub(super) fn pick_media_files(state: &SyncplayGuiShellAppState) -> Option<Vec<String>> {
        if let Some(paths) = Self::media_file_pick_override_paths_from_lookup(&env_trimmed) {
            return Some(paths);
        }
        let mut dialog = FileDialog::new().set_title("Select Media File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_files().map(|paths| {
            paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        })
    }

    pub(super) fn media_file_pick_override_paths_from_lookup<F>(lookup: &F) -> Option<Vec<String>>
    where
        F: Fn(&str) -> Option<String>,
    {
        let paths = lookup("SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS")?
            .split('|')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if paths.is_empty() { None } else { Some(paths) }
    }

    pub(super) fn pick_playlist_load_file(state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(path) = Self::playlist_load_override_path_from_lookup(&env_trimmed) {
            return Some(path);
        }
        let mut dialog = FileDialog::new().set_title("Load Playlist From File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .add_filter("playlist", &["txt", "m3u", "m3u8"])
            .pick_file()
            .map(|path| path.to_string_lossy().into_owned())
    }

    pub(super) fn playlist_load_override_path_from_lookup<F>(lookup: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        lookup("SYNCPLAY_GUI_TEST_LOAD_PLAYLIST_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    pub(super) fn pick_playlist_save_file(state: &SyncplayGuiShellAppState) -> Option<String> {
        if let Some(path) = Self::playlist_save_override_path_from_lookup(&env_trimmed) {
            return Some(path);
        }
        let mut dialog = FileDialog::new().set_title("Save Playlist To File");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .add_filter("playlist", &["txt", "m3u", "m3u8"])
            .save_file()
            .map(|path| path.to_string_lossy().into_owned())
    }

    pub(super) fn playlist_save_override_path_from_lookup<F>(lookup: &F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        lookup("SYNCPLAY_GUI_TEST_SAVE_PLAYLIST_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    pub(super) fn pick_media_search_directories(
        state: &SyncplayGuiShellAppState,
    ) -> Option<Vec<String>> {
        if let Some(paths) = Self::media_search_browse_override_paths_from_lookup(&env_trimmed) {
            return Some(paths);
        }
        let mut dialog = FileDialog::new().set_title("Select Media Search Directory");
        if let Some(directory) = Self::media_search_dialog_start_directory(state) {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .pick_folders()
            .map(|paths| {
                paths
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect()
            })
            .filter(|paths: &Vec<String>| !paths.is_empty())
    }

    pub(super) fn media_search_browse_override_paths_from_lookup<F>(
        lookup: &F,
    ) -> Option<Vec<String>>
    where
        F: Fn(&str) -> Option<String>,
    {
        let paths = lookup("SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH")?
            .split('|')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if paths.is_empty() { None } else { Some(paths) }
    }

    pub(super) fn media_search_dialog_start_directory(
        state: &SyncplayGuiShellAppState,
    ) -> Option<&str> {
        state.last_media_dialog_directory.as_deref().or_else(|| {
            state
                .selection
                .selected_media_search_directory
                .and_then(|index| state.media_search.directories.get(index))
                .or_else(|| state.media_search.directories.first())
                .map(|row| row.path.as_str())
        })
    }
}

fn hovered_playlist_insert_slot(
    playlist_drop_target_hovered: bool,
    playlist_drop_target_rect: Option<egui::Rect>,
    pointer_hover_pos: Option<egui::Pos2>,
    playlist_drop_target_slot: Option<usize>,
) -> Option<usize> {
    let hovered_playlist_target = playlist_drop_target_hovered
        || playlist_drop_target_rect
            .zip(pointer_hover_pos)
            .is_some_and(|(rect, pointer)| rect.contains(pointer));
    hovered_playlist_target
        .then_some(playlist_drop_target_slot)
        .flatten()
}
