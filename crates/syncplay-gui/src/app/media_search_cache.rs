use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION: u32 = 2;

static MEDIA_SEARCH_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);
static MEDIA_SEARCH_CACHE_IO_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedMediaSearchRootIndexV2 {
    pub(super) version: u32,
    pub(super) root_key: String,
    pub(super) root_path: String,
    pub(super) built_at_unix_ms: u64,
    pub(super) candidates_by_name: HashMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct PersistedMediaSearchRootIndexV2Ref<'a> {
    version: u32,
    root_key: &'a str,
    root_path: Cow<'a, str>,
    built_at_unix_ms: u64,
    candidates_by_name: &'a HashMap<String, Vec<String>>,
}

pub(super) fn normalized_media_search_root_key(path: &Path) -> String {
    normalized_media_search_root_path_string(path, true)
}

fn canonical_media_search_root_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn strip_windows_verbatim_prefix(path: String) -> String {
    if cfg!(windows) {
        if let Some(rest) = path.strip_prefix("//?/UNC/") {
            return format!("//{rest}");
        }
        if let Some(rest) = path.strip_prefix("//?/") {
            return rest.to_owned();
        }
    }
    path
}

fn normalized_media_search_root_path_string(path: &Path, lowercase_windows: bool) -> String {
    let mut key = canonical_media_search_root_path(path)
        .to_string_lossy()
        .replace('\\', "/");
    key = strip_windows_verbatim_prefix(key);
    while key.ends_with('/') && key.len() > 1 {
        key.pop();
    }
    if cfg!(windows) {
        if lowercase_windows {
            key.to_ascii_lowercase()
        } else {
            key
        }
    } else {
        key
    }
}

fn canonical_media_search_root_path_string(path: &Path) -> String {
    normalized_media_search_root_path_string(path, false)
}

pub(super) fn current_media_search_cache_generation() -> u64 {
    MEDIA_SEARCH_CACHE_GENERATION.load(Ordering::Acquire)
}

pub(super) fn current_unix_time_millis() -> u64 {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    milliseconds.min(u128::from(u64::MAX)) as u64
}

pub(super) fn persisted_media_search_cache_dir_at_root(gui_root: &Path) -> PathBuf {
    persisted_media_search_cache_root_at_root(gui_root).join("v2")
}

pub(super) fn persisted_media_search_cache_root_at_root(gui_root: &Path) -> PathBuf {
    gui_root.join("Syncplay").join("cache").join("media-search")
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

fn normalized_media_search_relative_candidate_key(candidate: &str) -> String {
    let mut normalized = candidate.replace('\\', "/");
    while normalized.ends_with('/') && normalized.len() > 1 {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn media_search_relative_candidate_depth(candidate: &str) -> usize {
    candidate
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .count()
}

fn media_search_candidate_is_safe_relative(root_key: &str, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let path = Path::new(candidate);
    if path.is_absolute() {
        return false;
    }
    let mut saw_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => saw_normal_component = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return false;
            }
        }
    }
    if !saw_normal_component {
        return false;
    }

    let candidate_key = normalized_media_search_relative_candidate_key(candidate);
    let root_prefix = format!("{root_key}/");
    candidate_key != root_key && !candidate_key.starts_with(&root_prefix)
}

fn sanitize_media_search_candidates_by_name(
    root_key: &str,
    candidates_by_name: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let mut sanitized = HashMap::new();
    for (name, candidates) in candidates_by_name {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let mut sanitized_candidates = candidates
            .iter()
            .map(|candidate| candidate.trim().to_owned())
            .filter(|candidate| media_search_candidate_is_safe_relative(root_key, candidate))
            .collect::<Vec<_>>();
        sanitized_candidates.sort_by_key(|candidate| {
            let lexical = normalized_media_search_relative_candidate_key(candidate);
            let depth = media_search_relative_candidate_depth(candidate);
            (depth, lexical)
        });
        sanitized_candidates.dedup_by(|left, right| {
            if cfg!(windows) {
                left.eq_ignore_ascii_case(right)
            } else {
                left == right
            }
        });
        if !sanitized_candidates.is_empty() {
            sanitized.insert(name.to_owned(), sanitized_candidates);
        }
    }
    sanitized
}

pub(super) fn load_persisted_media_search_root_index_at_root(
    gui_root: &Path,
    root_path: &Path,
) -> Result<Option<PersistedMediaSearchRootIndexV2>, String> {
    let root_key = normalized_media_search_root_key(root_path);
    let cache_path = persisted_media_search_root_index_path_at_root(gui_root, &root_key);
    let contents = match std::fs::read_to_string(&cache_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Ok(None),
    };
    let Ok(mut persisted) = serde_json::from_str::<PersistedMediaSearchRootIndexV2>(&contents)
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
    persisted.candidates_by_name =
        sanitize_media_search_candidates_by_name(&root_key, &persisted.candidates_by_name);
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
    // SAFETY: Both paths are converted from valid Windows `OsStr` values into
    // null-terminated UTF-16 buffers that live for the duration of the call.
    // The optional backup/exclusion/reserved pointers are intentionally null.
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
    // SAFETY: Both path buffers are null-terminated UTF-16 and remain alive for
    // the duration of the call. Flags request an atomic-ish replace fallback.
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

#[cfg(test)]
pub(super) fn persist_media_search_root_index_at_root(
    gui_root: &Path,
    index: &PersistedMediaSearchRootIndexV2,
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

#[cfg(test)]
pub(super) fn persist_media_search_root_index_borrowed_at_root<'a>(
    gui_root: &Path,
    root_key: &'a str,
    root_path: &'a Path,
    built_at_unix_ms: u64,
    candidates_by_name: &'a HashMap<String, Vec<String>>,
) -> Result<(), String> {
    persist_media_search_root_index_borrowed_at_root_checked(
        gui_root,
        root_key,
        root_path,
        built_at_unix_ms,
        candidates_by_name,
        None,
    )
}

pub(super) fn persist_media_search_root_index_borrowed_at_root_if_cache_generation(
    gui_root: &Path,
    root_key: &str,
    root_path: &Path,
    built_at_unix_ms: u64,
    candidates_by_name: &HashMap<String, Vec<String>>,
    expected_generation: u64,
) -> Result<(), String> {
    persist_media_search_root_index_borrowed_at_root_checked(
        gui_root,
        root_key,
        root_path,
        built_at_unix_ms,
        candidates_by_name,
        Some(expected_generation),
    )
}

fn persist_media_search_root_index_borrowed_at_root_checked(
    gui_root: &Path,
    root_key: &str,
    root_path: &Path,
    built_at_unix_ms: u64,
    candidates_by_name: &HashMap<String, Vec<String>>,
    expected_generation: Option<u64>,
) -> Result<(), String> {
    let normalized_root_key = normalized_media_search_root_key(root_path);
    if root_key != normalized_root_key {
        return Err(
            "persisted media-search cache write rejected inconsistent root metadata.".to_owned(),
        );
    }
    if expected_generation.is_some_and(|expected_generation| {
        current_media_search_cache_generation() != expected_generation
    }) {
        return Ok(());
    }
    let path = persisted_media_search_root_index_path_at_root(gui_root, root_key);
    let root_path_string = Cow::Owned(canonical_media_search_root_path_string(root_path));
    let sanitized_candidates =
        sanitize_media_search_candidates_by_name(root_key, candidates_by_name);
    let contents = serde_json::to_vec(&PersistedMediaSearchRootIndexV2Ref {
        version: PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION,
        root_key,
        root_path: root_path_string,
        built_at_unix_ms,
        candidates_by_name: &sanitized_candidates,
    })
    .map_err(|error| {
        format!(
            "failed encoding persisted media-search root index '{}': {error}",
            root_path.display()
        )
    })?;
    let _guard = MEDIA_SEARCH_CACHE_IO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if expected_generation.is_some_and(|expected_generation| {
        current_media_search_cache_generation() != expected_generation
    }) {
        return Ok(());
    }
    write_file_atomically(&path, &contents)
}

pub(super) fn clear_persisted_media_search_cache_at_root(gui_root: &Path) -> Result<(), String> {
    MEDIA_SEARCH_CACHE_GENERATION.fetch_add(1, Ordering::AcqRel);
    let _guard = MEDIA_SEARCH_CACHE_IO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache_root = persisted_media_search_cache_root_at_root(gui_root);
    if !cache_root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&cache_root).map_err(|error| {
        format!(
            "failed clearing persisted media-search cache '{}': {error}",
            cache_root.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::testing::support::test_temp_root;

    fn persisted_index(root: &Path, built_at_unix_ms: u64) -> PersistedMediaSearchRootIndexV2 {
        PersistedMediaSearchRootIndexV2 {
            version: PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION,
            root_key: normalized_media_search_root_key(root),
            root_path: canonical_media_search_root_path_string(root),
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
    fn persisted_media_search_root_index_uses_v2_cache_directory_and_ignores_v1() {
        let root = test_temp_root("media-search-cache-v2-only");
        let media_root = root.join("Media");
        let root_key = normalized_media_search_root_key(&media_root);
        let v1_path = persisted_media_search_cache_root_at_root(&root)
            .join("v1")
            .join(persisted_media_search_root_index_file_name(&root_key));
        std::fs::create_dir_all(
            v1_path
                .parent()
                .expect("v1 cache path should have a parent"),
        )
        .expect("v1 cache directory should be created");
        std::fs::write(
            &v1_path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "root_key": root_key,
                "root_path": media_root.to_string_lossy(),
                "built_at_unix_ms": 1234_u64,
                "candidates_by_name": { "episode1.mkv": ["episode1.mkv"] }
            }))
            .expect("v1 cache fixture should encode"),
        )
        .expect("v1 cache fixture should be written");

        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("v1 cache should be ignored cleanly")
                .is_none()
        );

        let index = persisted_index(&media_root, 1234);
        persist_media_search_root_index_at_root(&root, &index)
            .expect("v2 persisted media-search index should be written");
        assert!(persisted_media_search_cache_dir_at_root(&root).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persisted_media_search_root_index_canonicalizes_equivalent_existing_roots() {
        let root = test_temp_root("media-search-cache-canonical-root");
        let media_root = root.join("Media");
        std::fs::create_dir_all(&media_root).expect("media root should be created");
        let dotted_root = media_root.join(".");
        let index = persisted_index(&dotted_root, 1234);

        persist_media_search_root_index_at_root(&root, &index)
            .expect("canonical persisted media-search index should be written");
        let loaded = load_persisted_media_search_root_index_at_root(&root, &media_root)
            .expect("canonical equivalent root should load")
            .expect("canonical equivalent root should exist");

        assert_eq!(
            loaded.root_key,
            normalized_media_search_root_key(&media_root)
        );
        assert_eq!(
            loaded.root_path,
            canonical_media_search_root_path_string(&media_root)
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
    fn persisted_media_search_root_index_sanitizes_unsafe_candidates_on_load() {
        let root = test_temp_root("media-search-cache-unsafe-candidates");
        let media_root = root.join("Media");
        let root_key = normalized_media_search_root_key(&media_root);
        let cache_path = persisted_media_search_root_index_path_at_root(&root, &root_key);
        std::fs::create_dir_all(
            cache_path
                .parent()
                .expect("cache file should have a parent"),
        )
        .expect("persisted cache directory should be created");
        let unsafe_index = PersistedMediaSearchRootIndexV2 {
            version: PERSISTED_MEDIA_SEARCH_ROOT_INDEX_VERSION,
            root_key: root_key.clone(),
            root_path: media_root.to_string_lossy().into_owned(),
            built_at_unix_ms: 1234,
            candidates_by_name: HashMap::from([(
                "episode1.mkv".to_owned(),
                vec![
                    "".to_owned(),
                    "./episode1.mkv".to_owned(),
                    "../episode1.mkv".to_owned(),
                    "/episode1.mkv".to_owned(),
                    format!("{root_key}/episode1.mkv"),
                    "season-1/episode1.mkv".to_owned(),
                ],
            )]),
        };
        std::fs::write(
            &cache_path,
            serde_json::to_vec(&unsafe_index).expect("unsafe cache fixture should encode"),
        )
        .expect("unsafe cache fixture should be written");

        let loaded = load_persisted_media_search_root_index_at_root(&root, &media_root)
            .expect("unsafe persisted cache should load")
            .expect("unsafe persisted cache should exist");

        assert_eq!(
            loaded
                .candidates_by_name
                .get("episode1.mkv")
                .cloned()
                .unwrap_or_default(),
            vec!["season-1/episode1.mkv".to_owned()]
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

    #[test]
    fn persisted_media_search_root_index_rejects_stale_generation_writes_after_clear() {
        let root = test_temp_root("media-search-cache-stale-generation");
        let media_root = root.join("Media");
        let index = persisted_index(&media_root, 1234);
        let stale_generation = current_media_search_cache_generation();

        clear_persisted_media_search_cache_at_root(&root)
            .expect("clear should advance cache generation");
        persist_media_search_root_index_borrowed_at_root_if_cache_generation(
            &root,
            &index.root_key,
            Path::new(&index.root_path),
            index.built_at_unix_ms,
            &index.candidates_by_name,
            stale_generation,
        )
        .expect("stale generation write should be ignored without failing");

        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("stale generation write should not create cache")
                .is_none()
        );

        persist_media_search_root_index_at_root(&root, &index)
            .expect("current generation cache write should succeed");
        assert!(
            load_persisted_media_search_root_index_at_root(&root, &media_root)
                .expect("current generation cache should load")
                .is_some()
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
