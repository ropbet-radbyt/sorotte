use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::paths::{managed_stream_helper_bin_dir, managed_stream_helper_bin_dir_candidates};
use super::{ManagedStreamHelperMetadata, STREAM_HELPER_STALE_AFTER};

pub(in crate::app::stream_support) fn managed_installation_is_stale(
    metadata: Option<&ManagedStreamHelperMetadata>,
) -> bool {
    let Some(metadata) = metadata else {
        return true;
    };
    let Some(installed_at) = metadata.installed_at_unix_seconds else {
        return true;
    };
    current_unix_seconds().saturating_sub(installed_at) > STREAM_HELPER_STALE_AFTER.as_secs()
}

fn managed_stream_helper_metadata_path(root: &Path) -> PathBuf {
    managed_stream_helper_bin_dir(root).join("metadata.json")
}

pub(in crate::app::stream_support) fn load_managed_stream_helper_metadata(
    root: &Path,
) -> Option<ManagedStreamHelperMetadata> {
    managed_stream_helper_bin_dir_candidates(root)
        .into_iter()
        .map(|directory| directory.join("metadata.json"))
        .find_map(|path| {
            let contents = fs::read_to_string(path).ok()?;
            serde_json::from_str(&contents).ok()
        })
}

pub(in crate::app::stream_support) fn save_managed_stream_helper_metadata(
    root: &Path,
    metadata: &ManagedStreamHelperMetadata,
) -> Result<(), String> {
    let path = managed_stream_helper_metadata_path(root);
    let contents = serde_json::to_string_pretty(metadata)
        .map_err(|error| format!("failed to serialize stream-helper metadata: {error}"))?;
    fs::write(&path, contents).map_err(|error| {
        format!(
            "failed to write stream-helper metadata '{}': {error}",
            path.display()
        )
    })
}

pub(in crate::app::stream_support) fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
