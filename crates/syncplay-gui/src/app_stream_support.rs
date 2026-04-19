use std::{
    env, fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use super::shell_state::{
    GuiStreamHelperHealth, GuiStreamHelperRuntimeSnapshot, GuiStreamTargetKind,
    browser_stream_target_kind,
};

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

fn managed_stream_helper_storage_root(root: &Path) -> PathBuf {
    let already_scoped = root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Syncplay"));
    if already_scoped {
        root.to_path_buf()
    } else {
        root.join("Syncplay")
    }
}

fn legacy_managed_stream_helper_bin_dir(root: &Path) -> PathBuf {
    root.join("tools").join("stream-helper").join("bin")
}

pub(super) fn managed_stream_helper_bin_dir(root: &Path) -> PathBuf {
    managed_stream_helper_storage_root(root)
        .join("tools")
        .join("stream-helper")
        .join("bin")
}

pub(super) fn managed_stream_helper_downloader_path(root: &Path) -> PathBuf {
    managed_stream_helper_bin_dir(root).join(managed_downloader_file_name())
}

fn managed_stream_helper_bin_dir_candidates(root: &Path) -> Vec<PathBuf> {
    let canonical = managed_stream_helper_bin_dir(root);
    let legacy = legacy_managed_stream_helper_bin_dir(root);
    if canonical == legacy {
        vec![canonical]
    } else {
        vec![canonical, legacy]
    }
}

fn discovered_managed_stream_helper_bin_dir(root: &Path) -> Option<PathBuf> {
    managed_stream_helper_bin_dir_candidates(root)
        .into_iter()
        .find(|path| path.is_dir())
}

fn discover_managed_stream_helper_component(root: &Path, file_name: &str) -> Option<PathBuf> {
    managed_stream_helper_bin_dir_candidates(root)
        .into_iter()
        .map(|directory| directory.join(file_name))
        .find(|path| path.is_file())
}

pub(super) fn managed_stream_helper_path_prefixes(root: Option<&Path>) -> Vec<PathBuf> {
    let Some(root) = root else {
        return Vec::new();
    };
    managed_stream_helper_bin_dir_candidates(root)
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

pub(super) fn probe_stream_helper_runtime_snapshot(
    root: Option<&Path>,
    attach_mode: StreamHelperAttachMode,
    target: Option<&str>,
) -> GuiStreamHelperRuntimeSnapshot {
    let extractor_target = target.and_then(|target| {
        (browser_stream_target_kind(target, None) == GuiStreamTargetKind::ExtractorPageUrl)
            .then_some(target)
    });
    let install_supported = cfg!(windows) && root.is_some();
    let integration_supported = root.is_some();
    let discovery = discover_stream_helpers(root);
    let metadata = root.and_then(load_managed_stream_helper_metadata);
    let install_location = root.map(|root| {
        discovered_managed_stream_helper_bin_dir(root)
            .unwrap_or_else(|| managed_stream_helper_bin_dir(root))
            .display()
            .to_string()
    });
    let downloader_probe = probe_stream_helper_component(
        ManagedStreamHelperComponent::Downloader,
        attach_mode,
        discovery.managed_downloader.clone(),
        discovery.environment_downloader.clone(),
    );
    let js_runtime_probe = probe_stream_helper_component(
        ManagedStreamHelperComponent::JsRuntime,
        attach_mode,
        discovery.managed_js_runtime.clone(),
        discovery.environment_js_runtime.clone(),
    );
    let snapshot_details = StreamHelperRuntimeSnapshotDetails {
        install_location: install_location.clone(),
        downloader_status: Some(downloader_probe.status.clone()),
        js_runtime_status: Some(js_runtime_probe.status.clone()),
        open_install_location_available: root.is_some(),
    };
    let status_snapshot = |health: GuiStreamHelperHealth,
                           message: Option<String>,
                           target: Option<&str>,
                           retry_available: bool| {
        runtime_snapshot_with_details(
            health,
            message,
            target,
            install_supported,
            integration_supported,
            retry_available,
            snapshot_details.clone(),
        )
    };

    let Some(target) = extractor_target else {
        return status_snapshot(GuiStreamHelperHealth::Healthy, None, None, false);
    };

    if attach_mode == StreamHelperAttachMode::ExternalPlayer
        && (downloader_probe.effective_path.is_none() || js_runtime_probe.effective_path.is_none())
    {
        return status_snapshot(
            GuiStreamHelperHealth::ExternalPlayerUnmanaged,
            Some(
                "This URL needs yt-dlp and Deno to be visible to the already-running external mpv process. Install them globally or relaunch mpv from Syncplay after setup."
                    .to_owned(),
            ),
            Some(target),
            true,
        );
    }

    let Some(downloader_path) = downloader_probe.effective_path.clone() else {
        let health = if install_supported {
            GuiStreamHelperHealth::MissingDownloader
        } else {
            GuiStreamHelperHealth::UnsupportedPlatform
        };
        let message = if install_supported {
            "Extractor-backed page URLs need yt-dlp before mpv can load them. Import it or install the managed helper."
                .to_owned()
        } else {
            "Automatic helper installation is not available on this platform yet. Import yt-dlp and Deno or install them manually."
                .to_owned()
        };
        return status_snapshot(health, Some(message), Some(target), true);
    };
    if let Some(error) = downloader_probe.effective_error.clone() {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some(format!("yt-dlp could not be executed: {error}")),
            Some(target),
            true,
        );
    }

    let Some(js_runtime_path) = js_runtime_probe.effective_path.clone() else {
        let health = if install_supported {
            GuiStreamHelperHealth::MissingJsRuntime
        } else {
            GuiStreamHelperHealth::UnsupportedPlatform
        };
        let message = if install_supported {
            "This URL needs a JavaScript runtime for yt-dlp extraction. Import Deno or install the managed runtime."
                .to_owned()
        } else {
            "Automatic helper installation is not available on this platform yet. Import yt-dlp and Deno or install them manually."
                .to_owned()
        };
        return status_snapshot(health, Some(message), Some(target), true);
    };
    if let Some(error) = js_runtime_probe.effective_error.clone() {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some(format!("Deno could not be executed: {error}")),
            Some(target),
            true,
        );
    }
    let downloader = StreamHelperExecutable {
        path: downloader_path,
        source: downloader_probe
            .effective_source
            .expect("source should exist with the effective downloader path"),
        version: downloader_probe.effective_version.clone(),
    };
    let js_runtime = StreamHelperExecutable {
        path: js_runtime_path,
        source: js_runtime_probe
            .effective_source
            .expect("source should exist with the effective JS runtime path"),
        version: js_runtime_probe.effective_version.clone(),
    };

    let using_managed_installation = downloader.source == StreamHelperSource::Managed
        || js_runtime.source == StreamHelperSource::Managed;
    if using_managed_installation && managed_installation_is_stale(metadata.as_ref()) {
        return status_snapshot(
            GuiStreamHelperHealth::Stale,
            Some(format!(
                "Managed stream helper found at '{}' and '{}', but it should be refreshed before retrying this URL.",
                downloader.path.display(),
                js_runtime.path.display()
            )),
            Some(target),
            true,
        );
    }

    status_snapshot(GuiStreamHelperHealth::Healthy, None, Some(target), true)
}

pub(super) fn install_or_update_managed_stream_helper(root: &Path) -> Result<String, String> {
    install_or_update_managed_stream_helper_with_progress(root, |_| {})
}

pub(super) fn install_or_update_managed_stream_helper_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    if !cfg!(windows) {
        return Err(
            "Automatic stream-helper installation is only implemented for Windows in this release."
                .to_owned(),
        );
    }

    let bin_dir = managed_stream_helper_bin_dir(root);
    progress(StreamHelperRemediationProgress::new(
        "Preparing stream helper remediation",
        Some(format!(
            "Creating managed stream helper directory at '{}'.",
            bin_dir.display()
        )),
        0.08,
    ));
    fs::create_dir_all(&bin_dir).map_err(|error| {
        format!(
            "failed to create managed stream helper directory '{}': {error}",
            bin_dir.display()
        )
    })?;

    let client = helper_http_client()?;
    let yt_dlp_path = bin_dir.join(managed_downloader_file_name());
    let deno_path = bin_dir.join(managed_js_runtime_file_name());
    progress(StreamHelperRemediationProgress::new(
        "Downloading yt-dlp",
        Some(format!("Saving yt-dlp into '{}'.", yt_dlp_path.display())),
        0.25,
    ));
    download_to_path(&client, YTDLP_WINDOWS_LATEST_URL, &yt_dlp_path)?;
    progress(StreamHelperRemediationProgress::new(
        "Downloading Deno",
        Some(format!("Saving Deno into '{}'.", deno_path.display())),
        0.50,
    ));
    let deno_bytes = download_bytes(&client, &windows_deno_latest_url()?)?;
    extract_deno_executable_from_zip(&deno_bytes, &deno_path)?;

    progress(StreamHelperRemediationProgress::new(
        "Validating stream helper binaries",
        Some("Checking that yt-dlp and Deno can be executed.".to_owned()),
        0.72,
    ));
    let downloader_version = validate_installed_stream_helper_component(
        &yt_dlp_path,
        ManagedStreamHelperComponent::Downloader,
    )?;
    let js_runtime_version = validate_installed_stream_helper_component(
        &deno_path,
        ManagedStreamHelperComponent::JsRuntime,
    )?;
    progress(StreamHelperRemediationProgress::new(
        "Saving stream helper metadata",
        Some("Recording the installed helper versions for later health checks.".to_owned()),
        0.82,
    ));
    save_managed_stream_helper_metadata(
        root,
        &ManagedStreamHelperMetadata {
            installed_at_unix_seconds: Some(current_unix_seconds()),
            downloader_version: Some(downloader_version),
            js_runtime_version: Some(js_runtime_version),
        },
    )?;

    Ok(format!(
        "Installed managed stream helper into '{}'.",
        bin_dir.display()
    ))
}

pub(super) fn import_managed_stream_helper_downloader(
    root: &Path,
    source_path: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_downloader_with_progress(root, source_path, |_| {})
}

pub(super) fn import_managed_stream_helper_downloader_with_progress<F>(
    root: &Path,
    source_path: &Path,
    progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    import_managed_stream_helper_component(
        root,
        source_path,
        ManagedStreamHelperComponent::Downloader,
        progress,
    )
}

pub(super) fn import_managed_stream_helper_js_runtime(
    root: &Path,
    source_path: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_js_runtime_with_progress(root, source_path, |_| {})
}

pub(super) fn import_managed_stream_helper_js_runtime_with_progress<F>(
    root: &Path,
    source_path: &Path,
    progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    import_managed_stream_helper_component(
        root,
        source_path,
        ManagedStreamHelperComponent::JsRuntime,
        progress,
    )
}

fn runtime_snapshot_with_details(
    health: GuiStreamHelperHealth,
    message: Option<String>,
    target: Option<&str>,
    install_supported: bool,
    integration_supported: bool,
    retry_available: bool,
    details: StreamHelperRuntimeSnapshotDetails,
) -> GuiStreamHelperRuntimeSnapshot {
    GuiStreamHelperRuntimeSnapshot {
        health,
        message,
        target: target.map(str::to_owned),
        install_supported,
        integration_supported,
        retry_available,
        install_location: details.install_location,
        downloader_status: details.downloader_status,
        js_runtime_status: details.js_runtime_status,
        open_install_location_available: details.open_install_location_available,
    }
}

fn probe_stream_helper_component(
    component: ManagedStreamHelperComponent,
    attach_mode: StreamHelperAttachMode,
    managed: Option<PathBuf>,
    environment: Option<PathBuf>,
) -> StreamHelperComponentProbe {
    let describe_effective_path =
        |path: PathBuf, source: StreamHelperSource, source_label: &'static str| {
            match probe_executable_version(&path, &["--version"]) {
                Ok(version) => StreamHelperComponentProbe {
                    effective_path: Some(path.clone()),
                    effective_source: Some(source),
                    effective_version: Some(version.clone()),
                    effective_error: None,
                    status: format!("{source_label}: {version} ({})", path.display()),
                },
                Err(error) => StreamHelperComponentProbe {
                    effective_path: Some(path.clone()),
                    effective_source: Some(source),
                    effective_version: None,
                    effective_error: Some(error.clone()),
                    status: format!(
                        "{source_label} at '{}' is unusable: {error}",
                        path.display()
                    ),
                },
            }
        };

    match attach_mode {
        StreamHelperAttachMode::ManagedPlayer => managed
            .map(|path| {
                describe_effective_path(path, StreamHelperSource::Managed, "Managed install")
            })
            .or_else(|| {
                environment.map(|path| {
                    describe_effective_path(path, StreamHelperSource::Environment, "PATH")
                })
            })
            .unwrap_or_else(|| StreamHelperComponentProbe {
                effective_path: None,
                effective_source: None,
                effective_version: None,
                effective_error: None,
                status: format!(
                    "Missing from Syncplay's managed install and PATH for {}.",
                    component.display_name()
                ),
            }),
        StreamHelperAttachMode::ExternalPlayer => environment
            .map(|path| describe_effective_path(path, StreamHelperSource::Environment, "PATH"))
            .unwrap_or_else(|| {
                if let Some(path) = managed {
                    return StreamHelperComponentProbe {
                        effective_path: None,
                        effective_source: None,
                        effective_version: None,
                        effective_error: None,
                        status: format!(
                            "Managed install present at '{}', but an external mpv process can only use PATH-visible helpers.",
                            path.display()
                        ),
                    };
                }
                StreamHelperComponentProbe {
                    effective_path: None,
                    effective_source: None,
                    effective_version: None,
                    effective_error: None,
                    status: format!(
                        "Missing from PATH for the external player: {}.",
                        component.display_name()
                    ),
                }
            }),
    }
}

fn import_managed_stream_helper_component(
    root: &Path,
    source_path: &Path,
    component: ManagedStreamHelperComponent,
    mut progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    if !source_path.is_file() {
        return Err(format!(
            "{} import failed because '{}' is not a file.",
            component.display_name(),
            source_path.display()
        ));
    }

    let bin_dir = managed_stream_helper_bin_dir(root);
    progress(StreamHelperRemediationProgress::new(
        format!("Preparing {}", component.display_name()),
        Some(format!(
            "Copying '{}' into '{}'.",
            source_path.display(),
            bin_dir.display()
        )),
        0.12,
    ));
    fs::create_dir_all(&bin_dir).map_err(|error| {
        format!(
            "failed to create managed stream helper directory '{}': {error}",
            bin_dir.display()
        )
    })?;

    let target_path = bin_dir.join(component.target_file_name());
    progress(StreamHelperRemediationProgress::new(
        format!("Importing {}", component.display_name()),
        Some(format!("Writing '{}'.", target_path.display())),
        0.38,
    ));
    let version = if target_path == source_path {
        probe_executable_version(&target_path, &["--version"]).map_err(|error| {
            format!(
                "{} could not be executed from '{}': {error}",
                component.display_name(),
                target_path.display()
            )
        })?
    } else {
        replace_managed_helper_executable_from_path(source_path, &target_path).and_then(|_| {
            probe_executable_version(&target_path, &["--version"]).map_err(|error| {
                let _ = fs::remove_file(&target_path);
                format!(
                    "{} could not be executed after import to '{}': {error}",
                    component.display_name(),
                    target_path.display()
                )
            })
        })?
    };

    progress(StreamHelperRemediationProgress::new(
        format!("Validating {}", component.display_name()),
        Some("Checking that the imported helper binary can be executed.".to_owned()),
        0.64,
    ));
    let mut metadata = load_managed_stream_helper_metadata(root).unwrap_or_default();
    metadata.installed_at_unix_seconds = Some(current_unix_seconds());
    component.assign_version(&mut metadata, version);
    progress(StreamHelperRemediationProgress::new(
        "Saving stream helper metadata",
        Some("Updating the managed helper inventory after import.".to_owned()),
        0.78,
    ));
    save_managed_stream_helper_metadata(root, &metadata)?;

    Ok(format!(
        "Imported {} into '{}'.",
        component.display_name(),
        target_path.display()
    ))
}

fn validate_installed_stream_helper_component(
    path: &Path,
    component: ManagedStreamHelperComponent,
) -> Result<String, String> {
    probe_executable_version(path, &["--version"]).map_err(|error| {
        let _ = fs::remove_file(path);
        format!(
            "{} could not be executed after install to '{}': {error}",
            component.display_name(),
            path.display()
        )
    })
}

fn discover_stream_helpers(root: Option<&Path>) -> StreamHelperDiscovery {
    StreamHelperDiscovery {
        managed_downloader: root.and_then(|root| {
            discover_managed_stream_helper_component(root, managed_downloader_file_name())
        }),
        environment_downloader: find_executable_on_path(&[
            managed_downloader_file_name(),
            "yt-dlp",
            "youtube-dl.exe",
            "youtube-dl",
        ]),
        managed_js_runtime: root.and_then(|root| {
            discover_managed_stream_helper_component(root, managed_js_runtime_file_name())
        }),
        environment_js_runtime: find_executable_on_path(&[managed_js_runtime_file_name(), "deno"]),
    }
}

fn effective_helper_path(
    attach_mode: StreamHelperAttachMode,
    managed: Option<PathBuf>,
    environment: Option<PathBuf>,
) -> (Option<PathBuf>, Option<StreamHelperSource>) {
    match attach_mode {
        StreamHelperAttachMode::ManagedPlayer => managed
            .map(|path| (path, StreamHelperSource::Managed))
            .or_else(|| environment.map(|path| (path, StreamHelperSource::Environment)))
            .map_or((None, None), |(path, source)| (Some(path), Some(source))),
        StreamHelperAttachMode::ExternalPlayer => environment
            .map(|path| (path, StreamHelperSource::Environment))
            .map_or((None, None), |(path, source)| (Some(path), Some(source))),
    }
}

fn managed_installation_is_stale(metadata: Option<&ManagedStreamHelperMetadata>) -> bool {
    let Some(metadata) = metadata else {
        return true;
    };
    let Some(installed_at) = metadata.installed_at_unix_seconds else {
        return true;
    };
    current_unix_seconds().saturating_sub(installed_at) > STREAM_HELPER_STALE_AFTER.as_secs()
}

fn helper_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(STREAM_HELPER_DOWNLOAD_TIMEOUT)
        .user_agent(STREAM_HELPER_USER_AGENT)
        .build()
        .map_err(|error| format!("failed to build stream-helper HTTP client: {error}"))
}

fn download_bytes(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("failed to download '{url}': {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to download '{url}': HTTP {}",
            response.status()
        ));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("failed reading '{url}' response body: {error}"))
}

fn download_to_path(client: &Client, url: &str, path: &Path) -> Result<(), String> {
    let bytes = download_bytes(client, url)?;
    fs::write(path, bytes).map_err(|error| {
        format!(
            "failed to write downloaded stream helper file '{}': {error}",
            path.display()
        )
    })
}

fn replace_managed_helper_executable_from_path(
    source_path: &Path,
    target_path: &Path,
) -> Result<(), String> {
    let temp_path = target_path.with_extension(format!(
        "{}.importing",
        target_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("tmp")
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source_path, &temp_path).map_err(|error| {
        format!(
            "failed to copy '{}' into '{}': {error}",
            source_path.display(),
            temp_path.display()
        )
    })?;
    make_copied_helper_executable(&temp_path)?;
    if target_path.exists() {
        fs::remove_file(target_path).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!(
                "failed to replace existing managed helper '{}': {error}",
                target_path.display()
            )
        })?;
    }
    fs::rename(&temp_path, target_path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to move imported helper into '{}': {error}",
            target_path.display()
        )
    })
}

fn make_copied_helper_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .map_err(|error| {
                format!(
                    "failed to read helper permissions from '{}': {error}",
                    path.display()
                )
            })?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|error| {
            format!(
                "failed to mark imported helper '{}' as executable: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn extract_deno_executable_from_zip(bytes: &[u8], target_path: &Path) -> Result<(), String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)
        .map_err(|error| format!("failed to open downloaded Deno archive: {error}"))?;
    let mut found = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to read downloaded Deno archive entry: {error}"))?;
        let name = entry.name().to_ascii_lowercase();
        if !name.ends_with("deno.exe") {
            continue;
        }

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|error| format!("failed to extract Deno executable from archive: {error}"))?;
        fs::write(target_path, data).map_err(|error| {
            format!(
                "failed to write extracted Deno executable '{}': {error}",
                target_path.display()
            )
        })?;
        found = true;
        break;
    }
    if !found {
        return Err("downloaded Deno archive did not contain deno.exe".to_owned());
    }
    Ok(())
}

fn windows_deno_latest_url() -> Result<String, String> {
    let asset = match env::consts::ARCH {
        "x86_64" => "deno-x86_64-pc-windows-msvc.zip",
        "aarch64" => "deno-aarch64-pc-windows-msvc.zip",
        other => {
            return Err(format!(
                "automatic Deno installation is unsupported on Windows architecture '{other}'"
            ));
        }
    };
    Ok(format!(
        "https://github.com/denoland/deno/releases/latest/download/{asset}"
    ))
}

fn managed_stream_helper_metadata_path(root: &Path) -> PathBuf {
    managed_stream_helper_bin_dir(root).join("metadata.json")
}

fn load_managed_stream_helper_metadata(root: &Path) -> Option<ManagedStreamHelperMetadata> {
    managed_stream_helper_bin_dir_candidates(root)
        .into_iter()
        .map(|directory| directory.join("metadata.json"))
        .find_map(|path| {
            let contents = fs::read_to_string(path).ok()?;
            serde_json::from_str(&contents).ok()
        })
}

fn save_managed_stream_helper_metadata(
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

fn probe_executable_version(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(path)
        .args(args)
        .output()
        .map_err(|error| format!("failed to start '{}': {error}", path.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        return Err(detail);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(version)
}

fn managed_downloader_file_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

fn managed_js_runtime_file_name() -> &'static str {
    if cfg!(windows) { "deno.exe" } else { "deno" }
}

fn find_executable_on_path(candidates: &[&str]) -> Option<PathBuf> {
    let path_env = env::var_os("PATH")?;
    for directory in env::split_paths(&path_env) {
        for candidate in candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        GuiStreamHelperHealth, GuiStreamTargetKind, ManagedStreamHelperMetadata,
        STREAM_HELPER_STALE_AFTER, StreamHelperAttachMode, current_unix_seconds,
        import_managed_stream_helper_downloader, import_managed_stream_helper_js_runtime,
        load_managed_stream_helper_metadata, managed_installation_is_stale,
        managed_stream_helper_bin_dir, managed_stream_helper_path_prefixes,
        probe_stream_helper_runtime_snapshot, validate_installed_stream_helper_component,
    };

    fn version_capable_executable() -> std::path::PathBuf {
        [
            "python.exe",
            "python",
            "python3.exe",
            "python3",
            "pwsh.exe",
            "pwsh",
            "powershell.exe",
            "powershell",
            "node.exe",
            "node",
        ]
        .iter()
        .find_map(|candidate| super::find_executable_on_path(&[*candidate]))
        .expect("a version-capable executable should be available on PATH for stream-helper tests")
    }

    #[test]
    fn managed_stream_helper_path_prefixes_include_existing_bin_dir() {
        let root = std::env::temp_dir().join(format!(
            "syncplay-stream-helper-test-{}",
            std::process::id()
        ));
        let bin_dir = managed_stream_helper_bin_dir(&root);
        std::fs::create_dir_all(&bin_dir).expect("managed helper bin dir should be created");

        assert_eq!(
            managed_stream_helper_path_prefixes(Some(root.as_path())),
            vec![bin_dir.clone()]
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn probe_stream_helper_runtime_snapshot_ignores_direct_media_urls() {
        let snapshot = probe_stream_helper_runtime_snapshot(
            None,
            StreamHelperAttachMode::ManagedPlayer,
            Some("https://cdn.example.com/video.m3u8"),
        );

        assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
        assert_eq!(
            super::browser_stream_target_kind("https://cdn.example.com/video.m3u8", None,),
            GuiStreamTargetKind::DirectMediaUrl
        );
        assert_eq!(snapshot.message, None);
        assert_eq!(snapshot.target, None);
    }

    #[test]
    fn managed_installation_staleness_depends_on_metadata_age() {
        assert!(managed_installation_is_stale(None));
        assert!(managed_installation_is_stale(Some(
            &ManagedStreamHelperMetadata::default()
        )));

        let fresh_metadata = ManagedStreamHelperMetadata {
            installed_at_unix_seconds: Some(current_unix_seconds()),
            downloader_version: Some("test".to_owned()),
            js_runtime_version: Some("test".to_owned()),
        };
        assert!(!managed_installation_is_stale(Some(&fresh_metadata)));

        let stale_metadata = ManagedStreamHelperMetadata {
            installed_at_unix_seconds: Some(
                current_unix_seconds().saturating_sub(STREAM_HELPER_STALE_AFTER.as_secs() + 1),
            ),
            downloader_version: Some("test".to_owned()),
            js_runtime_version: Some("test".to_owned()),
        };
        assert!(managed_installation_is_stale(Some(&stale_metadata)));
    }

    #[test]
    fn importing_stream_helper_binaries_populates_managed_helper_paths_and_metadata() {
        let root = std::env::temp_dir().join(format!(
            "syncplay-stream-helper-import-{}-{}",
            std::process::id(),
            current_unix_seconds()
        ));
        let source_executable = version_capable_executable();

        let downloader_message = import_managed_stream_helper_downloader(&root, &source_executable)
            .expect("downloader import should succeed");
        assert!(downloader_message.contains("Imported yt-dlp"));
        assert!(
            managed_stream_helper_bin_dir(&root)
                .join(if cfg!(windows) {
                    "yt-dlp.exe"
                } else {
                    "yt-dlp"
                })
                .is_file()
        );

        let js_runtime_message = import_managed_stream_helper_js_runtime(&root, &source_executable)
            .expect("js-runtime import should succeed");
        assert!(js_runtime_message.contains("Imported Deno"));
        assert!(
            managed_stream_helper_bin_dir(&root)
                .join(if cfg!(windows) { "deno.exe" } else { "deno" })
                .is_file()
        );

        let metadata = load_managed_stream_helper_metadata(&root)
            .expect("managed helper metadata should be written");
        assert!(metadata.downloader_version.is_some());
        assert!(metadata.js_runtime_version.is_some());

        let snapshot = probe_stream_helper_runtime_snapshot(
            Some(root.as_path()),
            StreamHelperAttachMode::ManagedPlayer,
            Some("https://www.youtube.com/watch?v=UyjIPZfygTk"),
        );
        assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
        assert!(snapshot.integration_supported);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn installed_stream_helper_validation_rejects_unusable_binaries() {
        let root = std::env::temp_dir().join(format!(
            "syncplay-stream-helper-invalid-install-{}-{}",
            std::process::id(),
            current_unix_seconds()
        ));
        let path = managed_stream_helper_bin_dir(&root).join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        });
        std::fs::create_dir_all(path.parent().expect("managed helper dir should exist"))
            .expect("managed helper dir should be created");
        std::fs::write(&path, b"not an executable")
            .expect("invalid helper payload should be written");

        let error = validate_installed_stream_helper_component(
            &path,
            super::ManagedStreamHelperComponent::Downloader,
        )
        .expect_err("invalid helper payload should fail validation");
        assert!(error.contains("yt-dlp could not be executed after install"));
        assert!(
            !path.exists(),
            "failed install validation should remove the unusable helper payload"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
