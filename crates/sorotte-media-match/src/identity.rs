use std::path::{Component, Path};

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
            Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Component::RootDir => parts.push(String::new()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().to_ascii_lowercase()),
        }
    }
    let mut normalized = parts.join("/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
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
