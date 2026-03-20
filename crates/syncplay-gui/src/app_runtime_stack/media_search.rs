use std::{
    collections::HashMap,
    ffi::OsStr,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

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

    pub(in crate::app) fn missing_media_file_name_lookup_key(
        target_file_name: &str,
    ) -> Option<String> {
        let target_file_name = target_file_name.trim();
        let target_name = Path::new(target_file_name)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(target_file_name);
        if target_name.is_empty() {
            return None;
        }
        Some(if cfg!(windows) {
            target_name.to_ascii_lowercase()
        } else {
            target_name.to_owned()
        })
    }

    fn record_missing_media_index_entry(paths_by_name: &mut HashMap<String, String>, path: &Path) {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let Some(key) = Self::missing_media_file_name_lookup_key(file_name) else {
            return;
        };
        paths_by_name
            .entry(key)
            .or_insert_with(|| path.to_string_lossy().into_owned());
    }

    fn record_missing_media_index_directory_entry(
        paths_by_name: &mut HashMap<String, String>,
        directory: &Path,
        file_name: &OsStr,
    ) {
        let Some(file_name) = file_name.to_str() else {
            return;
        };
        let Some(key) = Self::missing_media_file_name_lookup_key(file_name) else {
            return;
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = paths_by_name.entry(key) {
            entry.insert(directory.join(file_name).to_string_lossy().into_owned());
        }
    }

    fn visit_missing_media_directory_entries<F>(
        directory: &Path,
        mut visitor: F,
    ) -> Result<(), String>
    where
        F: FnMut(&OsStr, bool, bool) -> bool,
    {
        #[cfg(windows)]
        {
            use std::{
                ffi::OsString,
                iter,
                os::windows::ffi::{OsStrExt, OsStringExt},
                ptr::null_mut,
            };

            use windows_sys::Win32::{
                Foundation::{ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE},
                Storage::FileSystem::{
                    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose,
                    FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
                    WIN32_FIND_DATAW,
                },
            };

            let search_pattern = directory.join("*");
            let mut search_pattern_wide = search_pattern
                .as_os_str()
                .encode_wide()
                .chain(iter::once(0))
                .collect::<Vec<_>>();
            let mut find_data = std::mem::MaybeUninit::<WIN32_FIND_DATAW>::zeroed();
            let handle = unsafe {
                FindFirstFileExW(
                    search_pattern_wide.as_mut_ptr(),
                    FindExInfoBasic,
                    find_data.as_mut_ptr().cast(),
                    FindExSearchNameMatch,
                    null_mut(),
                    FIND_FIRST_EX_LARGE_FETCH,
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(format!(
                    "Client-core session runtime could not scan '{}' during missing-media search: {}",
                    directory.display(),
                    std::io::Error::last_os_error()
                ));
            }

            struct FindHandleGuard(HANDLE);

            impl Drop for FindHandleGuard {
                fn drop(&mut self) {
                    unsafe {
                        FindClose(self.0);
                    }
                }
            }

            let _close_guard = FindHandleGuard(handle);
            loop {
                let current_find_data = unsafe { find_data.assume_init_ref() };
                let name_length = current_find_data
                    .cFileName
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(current_find_data.cFileName.len());
                let file_name = OsString::from_wide(&current_find_data.cFileName[..name_length]);
                if file_name != "." && file_name != ".." {
                    let is_dir =
                        (current_find_data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                    if !visitor(file_name.as_os_str(), is_dir, !is_dir) {
                        break;
                    }
                }

                let next_result = unsafe { FindNextFileW(handle, find_data.as_mut_ptr()) };
                if next_result != 0 {
                    continue;
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(format!(
                    "Client-core session runtime could not scan '{}' during missing-media search: {error}",
                    directory.display()
                ));
            }
            return Ok(());
        }

        #[cfg(not(windows))]
        {
            let entries = std::fs::read_dir(directory).map_err(|error| {
                format!(
                    "Client-core session runtime could not scan '{}' during missing-media search: {error}",
                    directory.display()
                )
            })?;
            for entry in entries {
                let Ok(entry) = entry else {
                    continue;
                };
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !visitor(
                    entry.file_name().as_os_str(),
                    file_type.is_dir(),
                    file_type.is_file(),
                ) {
                    break;
                }
            }
            Ok(())
        }
    }

    pub(in crate::app) fn build_missing_media_file_name_index_for_path(
        paths_by_name: &mut HashMap<String, String>,
        path: &Path,
        deadline: Option<Instant>,
        cancel_flag: &AtomicBool,
    ) -> Result<(), String> {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Client-core session runtime canceled missing-media indexing.".to_owned());
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("Client-core session runtime missing-media indexing timed out.".to_owned());
        }
        if path.is_file() {
            Self::record_missing_media_index_entry(paths_by_name, path);
            return Ok(());
        }

        if !path.is_dir() {
            return Ok(());
        }

        let mut pending_directories = vec![path.to_path_buf()];
        while let Some(directory) = pending_directories.pop() {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(
                    "Client-core session runtime canceled missing-media indexing.".to_owned(),
                );
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(
                    "Client-core session runtime missing-media indexing timed out.".to_owned(),
                );
            }
            Self::visit_missing_media_directory_entries(
                &directory,
                |file_name, is_dir, is_file| {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return false;
                    }
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        return false;
                    }
                    if is_dir {
                        pending_directories.push(directory.join(file_name));
                        return true;
                    }
                    if !is_file {
                        return true;
                    }
                    Self::record_missing_media_index_directory_entry(
                        paths_by_name,
                        &directory,
                        file_name,
                    );
                    true
                },
            )
            .map_err(|error| {
                error.replace(
                    "during missing-media search",
                    "during missing-media indexing",
                )
            })?;
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(
                    "Client-core session runtime canceled missing-media indexing.".to_owned(),
                );
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(
                    "Client-core session runtime missing-media indexing timed out.".to_owned(),
                );
            }
        }
        Ok(())
    }

    pub(in crate::app) fn search_path_for_missing_media_target(
        target_file_name: &str,
        path: &Path,
    ) -> Result<Option<String>, String> {
        let target_file_name = target_file_name.trim();
        let target_name = Path::new(target_file_name)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(target_file_name);
        if path.is_file() {
            let matches_target =
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|candidate| {
                        Self::missing_media_file_name_matches(target_name, candidate)
                    });
            if matches_target {
                return Ok(Some(path.to_string_lossy().into_owned()));
            }
            return Ok(None);
        }

        if !path.is_dir() {
            return Ok(None);
        }

        let mut pending_directories = vec![path.to_path_buf()];
        while let Some(directory) = pending_directories.pop() {
            for candidate in [
                directory.join(target_file_name),
                directory.join(target_name),
            ] {
                if candidate.is_file() {
                    return Ok(Some(candidate.to_string_lossy().into_owned()));
                }
            }

            let mut found_path = None;
            Self::visit_missing_media_directory_entries(
                &directory,
                |file_name, is_dir, is_file| {
                    if found_path.is_some() {
                        return false;
                    }
                    if is_dir {
                        pending_directories.push(directory.join(file_name));
                        return true;
                    }
                    if !is_file {
                        return true;
                    }
                    let matches_target = file_name.to_str().is_some_and(|candidate| {
                        Self::missing_media_file_name_matches(target_name, candidate)
                    });
                    if matches_target {
                        found_path = Some(directory.join(file_name).to_string_lossy().into_owned());
                        return false;
                    }
                    true
                },
            )?;
            if found_path.is_some() {
                return Ok(found_path);
            }
        }
        Ok(None)
    }
}
