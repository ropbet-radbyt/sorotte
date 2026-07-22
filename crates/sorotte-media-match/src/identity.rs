use std::{
    ffi::OsStr,
    path::{Component, Path},
};

use sha2::{Digest, Sha256};

pub(crate) fn duration_seconds_to_millis(duration_seconds: f64) -> Option<u32> {
    duration_seconds
        .is_finite()
        .then_some(duration_seconds)
        .filter(|value| *value >= 0.0)
        .map(|value| (value * 1000.0).round().min(f64::from(u32::MAX)) as u32)
}

pub fn normalize_media_path(path: impl AsRef<Path>) -> String {
    let mut parts = Vec::new();
    for component in path.as_ref().components() {
        match component {
            Component::Prefix(prefix) => parts.push(normalize_path_component(prefix.as_os_str())),
            Component::RootDir => parts.push(String::new()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::Normal(part) => parts.push(normalize_path_component(part)),
        }
    }
    let mut normalized = parts.join("/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

#[cfg(windows)]
fn normalize_path_component(component: &OsStr) -> String {
    component.to_string_lossy().to_ascii_lowercase()
}

#[cfg(not(windows))]
fn normalize_path_component(component: &OsStr) -> String {
    component.to_string_lossy().into_owned()
}

pub(crate) fn container_fingerprint_from_metadata(
    normalized_path: &str,
    modified_unix_millis: u64,
    size_bytes: u64,
    duration_seconds: Option<f64>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_path.as_bytes());
    hasher.update(modified_unix_millis.to_le_bytes());
    hasher.update(size_bytes.to_le_bytes());
    if let Some(duration_seconds) = duration_seconds {
        hasher.update(duration_seconds.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, time::SystemTime};

    use super::normalize_media_path;

    #[test]
    fn normalized_existing_mixed_case_path_remains_openable() {
        let root = std::env::temp_dir().join(format!(
            "Sorotte-Normalized-Path-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        let media_dir = root.join("MixedCaseLibrary");
        let media_path = media_dir.join("Show.S01E01.mkv");
        fs::create_dir_all(&media_dir).expect("mixed-case media directory should be created");
        fs::write(&media_path, b"fixture").expect("mixed-case media file should be written");

        let normalized = normalize_media_path(&media_path);

        assert!(
            Path::new(&normalized).is_file(),
            "normalized path must still address the existing file: {normalized}"
        );
        fs::remove_dir_all(root).expect("temporary media directory should be removable");
    }

    #[cfg(windows)]
    #[test]
    fn normalized_paths_are_case_insensitive_on_windows() {
        assert_eq!(
            normalize_media_path(r"C:\Media\Show.S01E01.mkv"),
            normalize_media_path(r"c:\media\show.s01e01.MKV")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn normalized_paths_keep_case_distinct_files_separate() {
        assert_ne!(
            normalize_media_path("/media/Show.S01E01.mkv"),
            normalize_media_path("/media/show.s01e01.mkv")
        );
    }
}
