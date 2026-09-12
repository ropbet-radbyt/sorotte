use std::path::{Path, PathBuf};

pub(in crate::app) fn managed_stream_helper_bin_dir(root: &Path) -> PathBuf {
    root.join("tools").join("stream-helper").join("bin")
}

pub(in crate::app) fn managed_stream_helper_downloader_path(root: &Path) -> PathBuf {
    managed_stream_helper_bin_dir(root).join(managed_downloader_file_name())
}

pub(in crate::app::stream_support) fn discovered_managed_stream_helper_bin_dir(
    root: &Path,
) -> Option<PathBuf> {
    Some(managed_stream_helper_bin_dir(root)).filter(|path| path.is_dir())
}

pub(in crate::app::stream_support) fn discover_managed_stream_helper_component(
    root: &Path,
    file_name: &str,
) -> Option<PathBuf> {
    Some(managed_stream_helper_bin_dir(root).join(file_name)).filter(|path| path.is_file())
}

pub(in crate::app) fn managed_stream_helper_path_prefixes(root: Option<&Path>) -> Vec<PathBuf> {
    let Some(root) = root else {
        return Vec::new();
    };
    discovered_managed_stream_helper_bin_dir(root)
        .into_iter()
        .collect()
}

pub(in crate::app::stream_support) fn managed_downloader_file_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

pub(in crate::app::stream_support) fn managed_js_runtime_file_name() -> &'static str {
    if cfg!(windows) { "deno.exe" } else { "deno" }
}
