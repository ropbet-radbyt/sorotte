use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn selected_opened_entry_offset(
        selected_playlist_index: Option<usize>,
        opened_entry_count: usize,
        playlist_insert_slot: Option<usize>,
    ) -> Option<usize> {
        let selected_index = selected_playlist_index?;
        if opened_entry_count == 0 {
            return None;
        }
        match playlist_insert_slot {
            Some(insert_slot) => selected_index
                .checked_sub(insert_slot)
                .filter(|offset| *offset < opened_entry_count),
            None => Some(selected_index).filter(|offset| *offset < opened_entry_count),
        }
    }

    pub(in crate::app::runtime_owner) fn open_media_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media requires a playback runtime connection; the selected file was not opened."
                .to_owned()
        } else {
            format!(
                "Opening media requires a playback runtime connection; {} selected files were not opened.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_open_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                .to_owned()
        } else {
            format!(
                "Opening media into the shared playlist requires a session or playback runtime connection; {} selected files were not opened or queued.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_session_unavailable_message_impl(&self) -> String {
        "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
            .to_owned()
    }

    pub(super) fn shared_playlist_control_unavailable_message_impl(&self) -> String {
        "Shared playlist control is unavailable for the active room; the selected media was not added to the room playlist or opened in the attached player."
            .to_owned()
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        let lower_path = path.to_ascii_lowercase();
        if !(lower_path.ends_with(".txt")
            || lower_path.ends_with(".m3u")
            || lower_path.ends_with(".m3u8"))
        {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = contents
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    pub(in crate::app::runtime_owner) fn shared_playlist_open_dispatch_for_paths_impl(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| shared_playlist_entry_for_media_path(path))
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
            imported_from_file: false,
        })
    }

    pub(super) fn shared_playlist_open_success_message(
        dispatch: &GuiSharedPlaylistOpenDispatch,
        entry_count: usize,
    ) -> String {
        if dispatch.imported_from_file {
            if entry_count == 1 {
                "Imported 1 entry into the shared playlist.".to_owned()
            } else {
                format!("Imported {entry_count} entries into the shared playlist.")
            }
        } else if entry_count == 1 {
            "Loaded 1 selected media entry into the shared playlist.".to_owned()
        } else {
            format!("Loaded {entry_count} selected media entries into the shared playlist.")
        }
    }
}
