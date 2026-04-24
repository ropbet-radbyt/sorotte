use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedMediaSearchRootIndexV1 {
    pub(super) version: u32,
    pub(super) root_key: String,
    pub(super) root_path: String,
    pub(super) built_at_unix_ms: u64,
    pub(super) candidates_by_name: HashMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct PersistedMediaSearchRootIndexV1Ref<'a> {
    version: u32,
    root_key: &'a str,
    root_path: Cow<'a, str>,
    built_at_unix_ms: u64,
    candidates_by_name: &'a HashMap<String, Vec<String>>,
}

pub(super) fn normalized_media_search_root_key(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

pub(super) fn current_unix_time_millis() -> u64 {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    milliseconds.min(u128::from(u64::MAX)) as u64
}

pub(super) fn persisted_media_search_cache_dir_at_root(gui_root: &Path) -> PathBuf {
    gui_root
        .join("Syncplay")
        .join("cache")
        .join("media-search")
        .join("v1")
}

fn persisted_media_search_root_index_file_name(root_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root_key.as_bytes());
    let digest = hasher.finalize();
    let mut file_name = String::with_capacity((digest.len() * 2) + 5);
    for byte in digest {
        use std::fmt::Write as _;

        let _ = write!(file_name, "{byte:02x}");
    }
    file_name.push_str(".json");
    file_name
}

fn persisted_media_search_root_index_path_at_root(gui_root: &Path, root_key: &str) -> PathBuf {
    persisted_media_search_cache_dir_at_root(gui_root)
        .join(persisted_media_search_root_index_file_name(root_key))
}

fn media_search_root_path_matches(expected_root_path: &Path, stored_root_path: &str) -> bool {
    let expected = normalized_media_search_root_key(expected_root_path);
    let stored = normalized_media_search_root_key(Path::new(stored_root_path));
    expected == stored
}

pub(super) fn load_persisted_media_search_root_index_at_root(
    gui_root: &Path,
    root_path: &Path,
) -> Result<Option<PersistedMediaSearchRootIndexV1>, String> {
    let root_key = normalized_media_search_root_key(root_path);
    let cache_path = persisted_media_search_root_index_path_at_root(gui_root, &root_key);
    let contents = match std::fs::read_to_string(&cache_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Ok(None),
    };
    let Ok(mut persisted) = serde_json::from_str::<PersistedMediaSearchRootIndexV1>(&contents)
    else {
        return Ok(None);
    };
    if persisted.version != PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION {
        return Ok(None);
    }
    if persisted.root_key != root_key
        || !media_search_root_path_matches(root_path, &persisted.root_path)
    {
        return Ok(None);
    }
    for candidates in persisted.candidates_by_name.values_mut() {
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
    persisted
        .candidates_by_name
        .retain(|_, candidates| !candidates.is_empty());
    Ok(Some(persisted))
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "persisted media-search cache path '{}' has no parent directory",
            path.display()
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed creating persisted media-search cache directory '{}': {error}",
            parent.display()
        )
    })?;
    let unique_suffix = current_unix_time_millis();
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("media-search-index"),
        unique_suffix
    ));
    {
        let mut file = std::fs::File::create(&temp_path).map_err(|error| {
            format!(
                "failed creating persisted media-search cache file '{}': {error}",
                temp_path.display()
            )
        })?;
        use std::io::Write as _;

        file.write_all(contents).map_err(|error| {
            format!(
                "failed writing persisted media-search cache file '{}': {error}",
                temp_path.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed flushing persisted media-search cache file '{}': {error}",
                temp_path.display()
            )
        })?;
    }

    let replace_result = replace_file_atomically(&temp_path, path);
    if replace_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    replace_result
}

#[cfg(windows)]
fn replace_file_atomically(from: &Path, to: &Path) -> Result<(), String> {
    use std::{iter, os::windows::ffi::OsStrExt, ptr::null_mut};

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    };

    if !to.exists() {
        return std::fs::rename(from, to).map_err(|error| {
            format!(
                "failed moving persisted media-search cache '{}' into '{}': {error}",
                from.display(),
                to.display()
            )
        });
    }

    let from_wide = from
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let to_wide = to
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            to_wide.as_ptr(),
            from_wide.as_ptr(),
            std::ptr::null(),
            0,
            null_mut(),
            null_mut(),
        )
    };
    if replaced != 0 {
        return Ok(());
    }

    let replace_error = std::io::Error::last_os_error();
    let moved = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(format!(
            "failed replacing persisted media-search cache '{}' with '{}': {}; fallback move failed: {}",
            to.display(),
            from.display(),
            replace_error,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_atomically(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|error| {
        format!(
            "failed moving persisted media-search cache '{}' into '{}': {error}",
            from.display(),
            to.display()
        )
    })
}

pub(super) fn persist_media_search_root_index_at_root(
    gui_root: &Path,
    index: &PersistedMediaSearchRootIndexV1,
) -> Result<(), String> {
    if index.version != PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION {
        return Err(
            "persisted media-search cache write rejected inconsistent root metadata.".to_owned(),
        );
    }
    persist_media_search_root_index_borrowed_at_root(
        gui_root,
        &index.root_key,
        Path::new(&index.root_path),
        index.built_at_unix_ms,
        &index.candidates_by_name,
    )
}

pub(super) fn persist_media_search_root_index_borrowed_at_root<'a>(
    gui_root: &Path,
    root_key: &'a str,
    root_path: &'a Path,
    built_at_unix_ms: u64,
    candidates_by_name: &'a HashMap<String, Vec<String>>,
) -> Result<(), String> {
    let normalized_root_key = normalized_media_search_root_key(root_path);
    if root_key != normalized_root_key {
        return Err(
            "persisted media-search cache write rejected inconsistent root metadata.".to_owned(),
        );
    }
    let path = persisted_media_search_root_index_path_at_root(gui_root, root_key);
    let root_path_string = root_path.to_string_lossy();
    let contents = serde_json::to_vec(&PersistedMediaSearchRootIndexV1Ref {
        version: PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION,
        root_key,
        root_path: root_path_string,
        built_at_unix_ms,
        candidates_by_name,
    })
    .map_err(|error| {
        format!(
            "failed encoding persisted media-search root index '{}': {error}",
            root_path.display()
        )
    })?;
    write_file_atomically(&path, &contents)
}

pub(super) fn clear_persisted_media_search_cache_at_root(gui_root: &Path) -> Result<(), String> {
    let cache_dir = persisted_media_search_cache_dir_at_root(gui_root);
    if !cache_dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed clearing persisted media-search cache '{}': {error}",
            cache_dir.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::testing::support::test_temp_root;

    fn persisted_index(root: &Path, built_at_unix_ms: u64) -> PersistedMediaSearchRootIndexV1 {
        PersistedMediaSearchRootIndexV1 {
            version: PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION,
            root_key: normalized_media_search_root_key(root),
            root_path: root.to_string_lossy().into_owned(),
            built_at_unix_ms,
            candidates_by_name: HashMap::from([(
                "episode1.mkv".to_owned(),
                vec![
                    "season-1\\episode1.mkv".to_owned(),
                    "season-1\\episode1.mkv".to_owned(),
                    "episode1.mkv".to_owned(),
                ],
            )]),
        }
    }

    #[test]
    fn persisted_media_search_root_index_round_trips() {
        let root = test_temp_root("media-search-cache-roundtrip");
        let media_root = root.join("Media");
        let index = persisted_index(&media_root, 1234);

        persist_media_search_root_index_at_root(&root, &index)
            .expect("persisted media-search index should be written");
        let loaded = load_persisted_media_search_root_index_at_root(&root, &media_root)
            .expect("persisted media-search index should load")
            .expect("persisted media-search index should exist");

        assert_eq!(loaded.version, index.version);
        assert_eq!(loaded.root_key, index.root_key);
        assert_eq!(loaded.root_path, index.root_path);
        assert_eq!(loaded.built_at_unix_ms, index.built_at_unix_ms);
        assert_eq!(
            loaded
                .candidates_by_name
                .get("episode1.mkv")
                .cloned()
                .unwrap_or_default(),
            vec![
                "episode1.mkv".to_owned(),
                "season-1\\episode1.mkv".to_owned()
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn borrowed_persisted_media_search_root_index_round_trips() {
        let root = test_temp_root("media-search-cache-borrowed-roundtrip");
        let media_root = root.join("Media");
        let candidates_by_name = HashMap::from([(
            "episode1.mkv".to_owned(),
            vec![
                "season-1\\episode1.mkv".to_owned(),
                "season-1\\episode1.mkv".to_owned(),
                "episode1.mkv".to_owned(),
            ],
        )]);

        persist_media_search_root_index_borrowed_at_root(
            &root,
            &normalized_media_search_root_key(&media_root),
            &media_root,
            1234,
            &candidates_by_name,
        )
        .expect("borrowed persisted media-search index should be written");

        let loaded = load_persisted_media_search_root_index_at_root(&root, &media_root)
            .expect("borrowed persisted media-search index should load")
            .expect("borrowed persisted media-search index should exist");

        assert_eq!(loaded.built_at_unix_ms, 1234);
        assert_eq!(
            loaded
                .candidates_by_name
                .get("episode1.mkv")
                .cloned()
                .unwrap_or_default(),
            vec![
                "episode1.mkv".to_owned(),
                "season-1\\episode1.mkv".to_owned()
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_media_search_root_index_ignores_corrupt_json() {
        let root = test_temp_root("media-search-cache-corrupt");
        let media_root = root.join("Media");
        let cache_path = persisted_media_search_root_index_path_at_root(
            &root,
            &normalized_media_search_root_key(&media_root),
        );
        std::fs::create_dir_all(
            cache_path
                .parent()
                .expect("cache file should have a parent"),
        )
        .expect("persisted cache directory should be created");
        std::fs::write(&cache_path, b"{not valid json")
            .expect("corrupt persisted cache should be written");

        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("corrupt persisted cache should be ignored")
                .is_none()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_media_search_root_index_ignores_version_mismatch() {
        let root = test_temp_root("media-search-cache-version");
        let media_root = root.join("Media");
        let mut index = persisted_index(&media_root, 1234);
        index.version += 1;
        let path = persisted_media_search_root_index_path_at_root(&root, &index.root_key);
        std::fs::create_dir_all(path.parent().expect("cache file should have a parent"))
            .expect("persisted cache directory should be created");
        std::fs::write(
            &path,
            serde_json::to_vec(&index).expect("mismatched version cache should encode"),
        )
        .expect("mismatched version cache should be written");

        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("mismatched version cache should be ignored")
                .is_none()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_media_search_root_index_ignores_wrong_root_path() {
        let root = test_temp_root("media-search-cache-root-mismatch");
        let media_root = root.join("Media");
        let mut index = persisted_index(&media_root, 1234);
        index.root_path = root.join("Other").to_string_lossy().into_owned();
        let path = persisted_media_search_root_index_path_at_root(&root, &index.root_key);
        std::fs::create_dir_all(path.parent().expect("cache file should have a parent"))
            .expect("persisted cache directory should be created");
        std::fs::write(
            &path,
            serde_json::to_vec(&index).expect("mismatched root cache should encode"),
        )
        .expect("mismatched root cache should be written");

        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("mismatched root cache should be ignored")
                .is_none()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_media_search_root_index_atomic_replace_preserves_valid_file() {
        let root = test_temp_root("media-search-cache-atomic-replace");
        let media_root = root.join("Media");
        let first = persisted_index(&media_root, 1111);
        let mut second = persisted_index(&media_root, 2222);
        second.candidates_by_name.insert(
            "episode2.mkv".to_owned(),
            vec!["season-2\\episode2.mkv".to_owned()],
        );

        persist_media_search_root_index_at_root(&root, &first)
            .expect("first persisted media-search index should be written");
        persist_media_search_root_index_at_root(&root, &second)
            .expect("second persisted media-search index should replace the first");

        let loaded = load_persisted_media_search_root_index_at_root(&root, &media_root)
            .expect("replaced persisted media-search index should load")
            .expect("replaced persisted media-search index should exist");
        assert_eq!(loaded.built_at_unix_ms, second.built_at_unix_ms);
        assert_eq!(
            loaded
                .candidates_by_name
                .get("episode2.mkv")
                .cloned()
                .unwrap_or_default(),
            vec!["season-2\\episode2.mkv".to_owned()]
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
