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

    fn record_missing_media_index_candidate(
        candidates_by_name: &mut HashMap<String, Vec<String>>,
        file_name: &str,
        relative_path: &Path,
    ) {
        let Some(key) = Self::missing_media_file_name_lookup_key(file_name) else {
            return;
        };
        let relative_path = relative_path.to_string_lossy().into_owned();
        if relative_path.trim().is_empty() {
            return;
        }
        candidates_by_name
            .entry(key)
            .or_default()
            .push(relative_path);
    }

    fn record_missing_media_index_file_entry(
        candidates_by_name: &mut HashMap<String, Vec<String>>,
        path: &Path,
    ) {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        Self::record_missing_media_index_candidate(
            candidates_by_name,
            file_name,
            Path::new(file_name),
        );
    }

    fn record_missing_media_index_directory_entry(
        candidates_by_name: &mut HashMap<String, Vec<String>>,
        root: &Path,
        directory: &Path,
        file_name: &OsStr,
    ) {
        let Some(file_name) = file_name.to_str() else {
            return;
        };
        let relative_path = match directory.strip_prefix(root) {
            Ok(relative_directory) if !relative_directory.as_os_str().is_empty() => {
                relative_directory.join(file_name)
            }
            _ => Path::new(file_name).to_path_buf(),
        };
        if relative_path.as_os_str().is_empty() {
            return;
        };
        Self::record_missing_media_index_candidate(candidates_by_name, file_name, &relative_path);
    }

    fn sort_missing_media_index_candidates(candidates: &mut Vec<String>) {
        candidates.retain(|candidate| !candidate.trim().is_empty());
        candidates.sort_by_key(|candidate| {
            let normalized = candidate.replace('\\', "/");
            let depth = Path::new(candidate).components().count();
            let lexical = if cfg!(windows) {
                normalized.to_ascii_lowercase()
            } else {
                normalized
            };
            (depth, lexical)
        });
        candidates.dedup_by(|left, right| {
            if cfg!(windows) {
                left.eq_ignore_ascii_case(right)
            } else {
                left == right
            }
        });
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

    pub(in crate::app) fn build_missing_media_file_name_index_for_path_with_progress<F>(
        path: &Path,
        deadline: Option<Instant>,
        cancel_flag: &AtomicBool,
        report_progress: &mut F,
    ) -> Result<HashMap<String, Vec<String>>, String>
    where
        F: FnMut(usize, usize),
    {
        let mut candidates_by_name = HashMap::new();
        let mut scanned_directories = 0usize;
        let mut indexed_files = 0usize;
        let mut last_reported_directories = 0usize;
        let mut last_reported_files = 0usize;
        report_progress(0, 0);
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Client-core session runtime canceled missing-media indexing.".to_owned());
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("Client-core session runtime missing-media indexing timed out.".to_owned());
        }
        if path.is_file() {
            Self::record_missing_media_index_file_entry(&mut candidates_by_name, path);
            report_progress(0, usize::from(!candidates_by_name.is_empty()));
            return Ok(candidates_by_name);
        }

        if !path.is_dir() {
            return Ok(candidates_by_name);
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
                        &mut candidates_by_name,
                        path,
                        &directory,
                        file_name,
                    );
                    indexed_files += 1;
                    if indexed_files.saturating_sub(last_reported_files) >= 128 {
                        report_progress(scanned_directories, indexed_files);
                        last_reported_directories = scanned_directories;
                        last_reported_files = indexed_files;
                    }
                    true
                },
            )
            .map_err(|error| {
                error.replace(
                    "during missing-media search",
                    "during missing-media indexing",
                )
            })?;
            scanned_directories += 1;
            if scanned_directories != last_reported_directories
                || indexed_files != last_reported_files
            {
                report_progress(scanned_directories, indexed_files);
                last_reported_directories = scanned_directories;
                last_reported_files = indexed_files;
            }
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
        for candidates in candidates_by_name.values_mut() {
            Self::sort_missing_media_index_candidates(candidates);
        }
        candidates_by_name.retain(|_, candidates| !candidates.is_empty());
        Ok(candidates_by_name)
    }

    pub(in crate::app) fn build_missing_media_file_name_index_for_path(
        path: &Path,
        deadline: Option<Instant>,
        cancel_flag: &AtomicBool,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        Self::build_missing_media_file_name_index_for_path_with_progress(
            path,
            deadline,
            cancel_flag,
            &mut |_, _| {},
        )
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
