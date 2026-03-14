use std::path::Path;

use super::GuiClientCoreChatSessionRuntimeAdapter;

impl GuiClientCoreChatSessionRuntimeAdapter {
    fn session_media_search_target(&self) -> Option<String> {
        if let Some(file_name) =
            self.runtime
                .session()
                .current_room_playlist()
                .and_then(|playlist| {
                    playlist
                        .index
                        .and_then(|index| usize::try_from(index).ok())
                        .and_then(|index| playlist.files.get(index))
                })
        {
            let trimmed = file_name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }

        if let Some(file_name) = self
            .runtime
            .session()
            .username
            .as_deref()
            .and_then(|username| self.runtime.session().user_file_name(username))
        {
            let trimmed = file_name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }

        self.tracked_remote_usernames.iter().find_map(|username| {
            self.runtime
                .session()
                .user_file_name(username)
                .map(str::trim)
                .filter(|file_name| !file_name.is_empty())
                .map(str::to_owned)
        })
    }

    pub(super) fn missing_media_search_target_file_name(&self) -> Result<String, String> {
        let Some(target) = self.session_media_search_target() else {
            return Err(
                "Client-core session runtime cannot search missing media because the current session does not expose a target file."
                    .to_owned(),
            );
        };
        if target.contains("://") {
            return Err(
                "Client-core session runtime cannot search missing media for URL-based media targets."
                    .to_owned(),
            );
        }
        let Some(file_name) = Path::new(&target)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return Err(
                "Client-core session runtime could not derive a file name for missing-media search."
                    .to_owned(),
            );
        };
        let file_name = file_name.trim();
        if file_name.is_empty() {
            return Err(
                "Client-core session runtime could not derive a non-empty file name for missing-media search."
                    .to_owned(),
            );
        }
        Ok(file_name.to_owned())
    }

    fn missing_media_file_name_matches(target: &str, candidate: &str) -> bool {
        if cfg!(windows) {
            candidate.eq_ignore_ascii_case(target)
        } else {
            candidate == target
        }
    }

    pub(in crate::app) fn search_path_for_missing_media_target(
        target_file_name: &str,
        path: &Path,
    ) -> Result<Option<String>, String> {
        if path.is_file() {
            let matches_target =
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|candidate| {
                        Self::missing_media_file_name_matches(target_file_name, candidate)
                    });
            if matches_target {
                return Ok(Some(path.to_string_lossy().into_owned()));
            }
            return Ok(None);
        }

        if !path.is_dir() {
            return Ok(None);
        }

        let mut children = std::fs::read_dir(path)
            .map_err(|error| {
                format!(
                    "Client-core session runtime could not scan '{}' during missing-media search: {error}",
                    path.display()
                )
            })?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect::<Vec<_>>();
        children.sort();

        for child in children {
            if let Some(found_path) =
                Self::search_path_for_missing_media_target(target_file_name, &child)?
            {
                return Ok(Some(found_path));
            }
        }
        Ok(None)
    }
}
