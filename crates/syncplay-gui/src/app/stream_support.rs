use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

mod discovery;
mod install;
mod metadata;
mod paths;
mod process;
mod snapshot;

#[cfg(test)]
mod tests;

pub(super) use install::{
    import_managed_stream_helper_downloader_with_progress,
    import_managed_stream_helper_js_runtime_with_progress,
    install_or_update_managed_stream_helper_with_progress,
};
use paths::{managed_downloader_file_name, managed_js_runtime_file_name};
pub(super) use paths::{
    managed_stream_helper_bin_dir, managed_stream_helper_downloader_path,
    managed_stream_helper_path_prefixes,
};
pub(super) use snapshot::probe_stream_helper_runtime_snapshot;

const STREAM_HELPER_STALE_AFTER: Duration = Duration::from_secs(30 * 86_400);
const STREAM_HELPER_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const STREAM_HELPER_USER_AGENT: &str = concat!("syncplay-rs-gui/", env!("CARGO_PKG_VERSION"));
const YTDLP_WINDOWS_LATEST_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamHelperAttachMode {
    ManagedPlayer,
    ExternalPlayer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamHelperSource {
    Managed,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamHelperExecutable {
    path: PathBuf,
    source: StreamHelperSource,
    version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamHelperComponentProbe {
    effective_path: Option<PathBuf>,
    effective_source: Option<StreamHelperSource>,
    effective_version: Option<String>,
    effective_error: Option<String>,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamHelperDiscovery {
    managed_downloader: Option<PathBuf>,
    environment_downloader: Option<PathBuf>,
    managed_js_runtime: Option<PathBuf>,
    environment_js_runtime: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamHelperRuntimeSnapshotDetails {
    install_location: Option<String>,
    downloader_status: Option<String>,
    js_runtime_status: Option<String>,
    open_install_location_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct ManagedStreamHelperMetadata {
    installed_at_unix_seconds: Option<u64>,
    downloader_version: Option<String>,
    js_runtime_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedStreamHelperComponent {
    Downloader,
    JsRuntime,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct StreamHelperRemediationProgress {
    pub(super) label: String,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
}

impl StreamHelperRemediationProgress {
    fn new(label: impl Into<String>, detail: Option<String>, progress_fraction: f32) -> Self {
        Self {
            label: label.into(),
            detail,
            progress_fraction,
        }
    }
}

impl ManagedStreamHelperComponent {
    fn display_name(self) -> &'static str {
        match self {
            Self::Downloader => "yt-dlp",
            Self::JsRuntime => "Deno",
        }
    }

    fn target_file_name(self) -> &'static str {
        match self {
            Self::Downloader => managed_downloader_file_name(),
            Self::JsRuntime => managed_js_runtime_file_name(),
        }
    }

    fn assign_version(self, metadata: &mut ManagedStreamHelperMetadata, version: String) {
        match self {
            Self::Downloader => metadata.downloader_version = Some(version),
            Self::JsRuntime => metadata.js_runtime_version = Some(version),
        }
    }
}
