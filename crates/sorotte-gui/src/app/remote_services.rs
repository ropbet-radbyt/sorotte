use std::{
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;
use sha2::{Digest, Sha256};
use sorotte_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;
use sorotte_client_app::app_boundary::persistence::parse_serialized_public_servers_list_legacy_compatible;
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use zip::ZipArchive;

use super::child_process::configure_gui_child_process;

const LEGACY_SYNCPLAY_VERSION: &str = "1.7.5";

fn lowercase_hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
const LEGACY_SYNCPLAY_MILESTONE: &str = "Yoitsu";
const LEGACY_SYNCPLAY_RELEASE_NUMBER: &str = "116";
const LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS: u64 = 86_400;
#[cfg(test)]
const LEGACY_SYNCPLAY_VERSION_STATUS_UP_TO_DATE: &str = "uptodate";
#[cfg(test)]
const LEGACY_SYNCPLAY_VERSION_STATUS_UPDATE_AVAILABLE: &str = "updateavailale";
const SYNCPLAY_PUBLIC_SERVER_LIST_URL: &str = "https://syncplay.pl/listpublicservers";
#[cfg(test)]
const SYNCPLAY_DOWNLOAD_URL: &str = "https://syncplay.pl/download/";
const GITHUB_RELEASES_PAGE_URL: &str = "https://github.com/ropbet-radbyt/sorotte/releases";
const GITHUB_RELEASE_LATEST_URL: &str =
    "https://api.github.com/repos/ropbet-radbyt/sorotte/releases/latest";
const GITHUB_DEV_RELEASE_URL: &str =
    "https://api.github.com/repos/ropbet-radbyt/sorotte/releases/tags/sorotte-gui-dev";
const SOROTTE_GUI_APP_NAME: &str = "sorotte-gui";
const SOROTTE_UPDATE_MANIFEST_NAME: &str = "sorotte-update-manifest.json";
const SOROTTE_GUI_TARGET: &str = "windows-x86_64";
const SOROTTE_GUI_RELEASE_PACKAGE_SUFFIX: &str = "-windows-x86_64.zip";
const SOROTTE_GUI_DEV_ARTIFACT_NAME: &str = "sorotte-gui-windows-x86_64";
const SOROTTE_GUI_INSTALL_MARKER: &str = "sorotte-install.json";
const SOROTTE_GUI_EXECUTABLE: &str = "sorotte-gui.exe";
const SOROTTE_GUI_UPDATER_EXECUTABLE: &str = "sorotte-gui-updater.exe";
#[cfg(windows)]
const SOROTTE_GUI_UPDATE_JOURNAL: &str = ".sorotte-update-journal-v1.jsonl";
const SOROTTE_PUBLIC_SERVER_LIST_URL_ENV: &str = "SOROTTE_GUI_PUBLIC_SERVER_LIST_URL";
const SOROTTE_PUBLIC_SERVER_LIST_RESPONSE_ENV: &str = "SOROTTE_GUI_PUBLIC_SERVER_LIST_RESPONSE";
const SOROTTE_UPDATE_CHECK_RESPONSE_ENV: &str = "SOROTTE_GUI_UPDATE_CHECK_RESPONSE";
const SOROTTE_GITHUB_RELEASE_LATEST_URL_ENV: &str = "SOROTTE_GUI_GITHUB_RELEASE_LATEST_URL";
const SOROTTE_GITHUB_DEV_RELEASE_URL_ENV: &str = "SOROTTE_GUI_GITHUB_DEV_RELEASE_URL";
const SOROTTE_GITHUB_ARTIFACTS_URL_ENV: &str = "SOROTTE_GUI_GITHUB_ARTIFACTS_URL";
const SOROTTE_GUI_UPDATE_CHANNEL_ENV: &str = "SOROTTE_GUI_UPDATE_CHANNEL";
const SOROTTE_GUI_GITHUB_TOKEN_ENV: &str = "SOROTTE_GUI_GITHUB_TOKEN";
const SOROTTE_GUI_BUILD_GIT_SHA_ENV: &str = "SOROTTE_GUI_BUILD_GIT_SHA";
const SOROTTE_GUI_BUILD_CREATED_AT_UTC_ENV: &str = "SOROTTE_GUI_BUILD_CREATED_AT_UTC";
static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "legacy parser tests still exercise unknown wire statuses"
)]
pub(crate) enum LegacyUpdateCheckStatus {
    UpToDate,
    UpdateAvailable,
    Checking,
    Failed,
    Unknown(String),
}

impl LegacyUpdateCheckStatus {
    #[cfg(test)]
    fn from_legacy_wire_value(value: &str) -> Self {
        match value.trim() {
            LEGACY_SYNCPLAY_VERSION_STATUS_UP_TO_DATE => Self::UpToDate,
            LEGACY_SYNCPLAY_VERSION_STATUS_UPDATE_AVAILABLE => Self::UpdateAvailable,
            "failed" => Self::Failed,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UpdateChannel {
    Stable,
    Dev,
}

impl UpdateChannel {
    fn selected(configured_channel: Option<&str>) -> Result<Self, String> {
        if let Some(value) = env_trimmed(SOROTTE_GUI_UPDATE_CHANNEL_ENV) {
            return Self::from_config_value(&value)
                .map_err(|error| format!("{SOROTTE_GUI_UPDATE_CHANNEL_ENV} {error}"));
        }
        if let Some(value) = configured_channel
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Self::from_config_value(value);
        }
        Ok(current_install_marker()
            .and_then(|marker| marker.channel)
            .unwrap_or(Self::Stable))
    }

    fn from_config_value(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" => Ok(Self::Stable),
            "dev" => Ok(Self::Dev),
            other => Err(format!("must be stable or dev, got {other:?}")),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Dev => "dev",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateCandidateSource {
    ReleaseAsset,
    ActionsArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UpdateManifest {
    pub(crate) schema: String,
    pub(crate) app: String,
    pub(crate) channel: UpdateChannel,
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) git_sha: Option<String>,
    pub(crate) created_at_utc: String,
    pub(crate) target: String,
    pub(crate) package: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GuiInstallMarker {
    #[serde(default)]
    channel: Option<UpdateChannel>,
    #[serde(default)]
    git_sha: Option<String>,
    #[serde(default)]
    created_at_utc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateCandidate {
    pub(crate) channel: UpdateChannel,
    pub(crate) version: String,
    pub(crate) git_sha: Option<String>,
    pub(crate) created_at_utc: String,
    pub(crate) target: String,
    pub(crate) package: String,
    pub(crate) sha256: String,
    pub(crate) download_url: String,
    pub(crate) details_url: Option<String>,
    pub(crate) source: UpdateCandidateSource,
}

impl UpdateCandidate {
    pub(crate) fn summary(&self) -> String {
        match self.channel {
            UpdateChannel::Stable => format!("Sorotte GUI {} is available.", self.version),
            UpdateChannel::Dev => {
                let sha = self
                    .git_sha
                    .as_deref()
                    .map(short_git_sha)
                    .unwrap_or("unknown");
                format!("A newer Sorotte GUI dev build is available ({sha}).")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum UpdateDownloadState {
    #[default]
    Idle,
    Downloading,
    Staged,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedUpdate {
    pub(crate) candidate: UpdateCandidate,
    pub(crate) package_path: String,
    pub(crate) source_dir: String,
    pub(crate) updater_path: String,
    pub(crate) target_exe_path: String,
    pub(crate) backup_dir: String,
    pub(crate) log_path: String,
    pub(crate) restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateDownloadResult {
    pub(crate) state: UpdateDownloadState,
    pub(crate) message: String,
    pub(crate) staged_update: Option<StagedUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateApplyLaunchResult {
    pub(crate) success: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyUpdateCheckResult {
    pub(crate) status: LegacyUpdateCheckStatus,
    pub(crate) message: String,
    pub(crate) url: Option<String>,
    pub(crate) candidate: Option<UpdateCandidate>,
    pub(crate) self_update_supported: bool,
    pub(crate) public_servers: Option<Vec<(String, String)>>,
    pub(crate) checked_at_utc: String,
    pub(crate) user_initiated: bool,
}

pub(crate) fn fetch_public_servers(
    language: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    if let Some(body) = env_response_override(SOROTTE_PUBLIC_SERVER_LIST_RESPONSE_ENV) {
        return parse_public_server_response(&body).map_err(|error| {
            format!(
                "{}\n-----\n{}",
                error,
                public_server_list_failed_message(language)
            )
        });
    }
    let url = std::env::var(SOROTTE_PUBLIC_SERVER_LIST_URL_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| SYNCPLAY_PUBLIC_SERVER_LIST_URL.to_owned());
    fetch_public_servers_from_url(&url, language).map_err(|error| {
        format!(
            "{}\n-----\n{}",
            error,
            public_server_list_failed_message(language)
        )
    })
}

fn fetch_public_servers_from_url(
    url: &str,
    language: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let language = normalized_language(language);
    let client = http_client()
        .map_err(|error| format!("failed to build public-server HTTP client: {error}"))?;

    let response = client
        .get(url)
        .query(&[
            ("version", LEGACY_SYNCPLAY_VERSION),
            ("milestone", LEGACY_SYNCPLAY_MILESTONE),
            ("release_number", LEGACY_SYNCPLAY_RELEASE_NUMBER),
            ("language", language),
        ])
        .send()
        .map_err(|error| format!("failed to load public server list: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "failed to load public server list: HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|error| format!("failed to read public server list response: {error}"))?;
    parse_public_server_response(&body)
}

fn parse_public_server_response(body: &str) -> Result<Vec<(String, String)>, String> {
    let normalized = sanitize_wordpress_public_server_response(body);
    let Some(rows) = parse_serialized_public_servers_list_legacy_compatible(&normalized) else {
        return Err(
            "failed to parse public server list response from the Syncplay service".to_owned(),
        );
    };
    if rows.is_empty() {
        return Err(
            "failed to load public server list: the Syncplay service returned no servers"
                .to_owned(),
        );
    }
    Ok(rows)
}

pub(crate) fn check_for_updates(
    language: Option<&str>,
    user_initiated: bool,
    update_channel: Option<&str>,
) -> LegacyUpdateCheckResult {
    let checked_at_utc =
        legacy_utc_timestamp_string_legacy_compatible(std::time::SystemTime::now());
    match check_for_github_update(language, user_initiated, update_channel) {
        Ok(result) => LegacyUpdateCheckResult {
            checked_at_utc,
            user_initiated,
            ..result
        },
        Err(error) => LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::Failed,
            message: format!(
                "{error}\n-----\n{}",
                github_update_check_failed_message(language)
            ),
            url: Some(GITHUB_RELEASES_PAGE_URL.to_owned()),
            candidate: None,
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc,
            user_initiated,
        },
    }
}

pub(crate) fn download_and_stage_update(
    candidate: &UpdateCandidate,
    gui_config_root: Option<&Path>,
) -> UpdateDownloadResult {
    match download_and_stage_update_inner(candidate, gui_config_root) {
        Ok(staged_update) => UpdateDownloadResult {
            state: UpdateDownloadState::Staged,
            message: "Update downloaded and staged. Restart Sorotte to apply it.".to_owned(),
            staged_update: Some(staged_update),
        },
        Err(error) => UpdateDownloadResult {
            state: UpdateDownloadState::Failed,
            message: error,
            staged_update: None,
        },
    }
}

pub(crate) fn launch_staged_update(staged_update: &StagedUpdate) -> UpdateApplyLaunchResult {
    match launch_staged_update_inner(staged_update) {
        Ok(()) => UpdateApplyLaunchResult {
            success: true,
            message: "Update helper started. Sorotte will close and restart after replacement."
                .to_owned(),
        },
        Err(error) => UpdateApplyLaunchResult {
            success: false,
            message: error,
        },
    }
}

#[cfg(windows)]
pub(crate) fn launch_pending_update_recovery() -> Result<bool, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current GUI executable: {error}"))?;
    let target_dir = current_exe
        .parent()
        .ok_or_else(|| "current GUI executable has no parent directory".to_owned())?;
    let journal_path = target_dir.join(SOROTTE_GUI_UPDATE_JOURNAL);
    match fs::symlink_metadata(&journal_path) {
        Ok(metadata) if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) => {
            return Err(format!(
                "pending update journal is not a regular file: {}",
                journal_path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed inspecting pending update journal {}: {error}",
                journal_path.display()
            ));
        }
    }

    let updater_path = target_dir.join(SOROTTE_GUI_UPDATER_EXECUTABLE);
    match fs::symlink_metadata(&updater_path) {
        Ok(metadata) if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) => {}
        Ok(_) => {
            return Err(format!(
                "installed update recovery helper is not a regular file: {}",
                updater_path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "failed inspecting installed update recovery helper {}: {error}",
                updater_path.display()
            ));
        }
    }

    let pid = std::process::id().to_string();
    let log_path = std::env::temp_dir().join(format!("sorotte-gui-update-recovery-{pid}.log"));
    let mut command = Command::new(&updater_path);
    command.args(pending_update_recovery_args(target_dir, &pid, &log_path));
    configure_gui_child_process(&mut command);
    command
        .spawn()
        .map_err(|error| format!("failed to launch interrupted-update recovery: {error}"))?;
    Ok(true)
}

#[cfg(not(windows))]
pub(crate) fn launch_pending_update_recovery() -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
fn pending_update_recovery_args(target_dir: &Path, pid: &str, log_path: &Path) -> Vec<String> {
    vec![
        "--recover".to_owned(),
        "--pid".to_owned(),
        pid.to_owned(),
        "--target-dir".to_owned(),
        target_dir.display().to_string(),
        "--target-exe".to_owned(),
        SOROTTE_GUI_EXECUTABLE.to_owned(),
        "--log".to_owned(),
        log_path.display().to_string(),
        "--restart".to_owned(),
    ]
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn cleanup_update_staging_root(gui_config_root: Option<&Path>) -> Result<(), String> {
    let Some(gui_config_root) = gui_config_root else {
        return Ok(());
    };
    let updates_root = gui_config_root.join("updates");
    match fs::symlink_metadata(&updates_root) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() => {
            return Err(format!(
                "update staging path is not a regular directory: {}",
                updates_root.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed inspecting update staging directory {}: {error}",
                updates_root.display()
            ));
        }
    }
    cleanup_updates_root_entries(&updates_root, None)?;
    match fs::remove_dir(&updates_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(format!(
            "failed to remove update staging directory {}: {error}",
            updates_root.display()
        )),
    }
}

fn check_for_github_update(
    language: Option<&str>,
    _user_initiated: bool,
    update_channel: Option<&str>,
) -> Result<LegacyUpdateCheckResult, String> {
    let capability = self_update_capability_current_install();
    if let Some(body) = env_response_override(SOROTTE_UPDATE_CHECK_RESPONSE_ENV)
        && let Some(result) = github_update_response_override_result(&body, language)?
    {
        return Ok(apply_self_update_capability(result, capability));
    }

    let channel = UpdateChannel::selected(update_channel)?;
    let result = match channel {
        UpdateChannel::Stable => check_stable_release_update(language),
        UpdateChannel::Dev => check_dev_update(language),
    }?;
    Ok(apply_self_update_capability(result, capability))
}

fn apply_self_update_capability(
    mut result: LegacyUpdateCheckResult,
    capability: SelfUpdateCapability,
) -> LegacyUpdateCheckResult {
    result.self_update_supported = capability.supported();
    if !capability.supported()
        && result.status == LegacyUpdateCheckStatus::UpdateAvailable
        && !result.message.contains(capability.unavailable_message())
    {
        result.message = format!(
            "{} {}",
            result.message.trim(),
            capability.unavailable_message()
        );
    }
    result
}

fn check_stable_release_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    let client = github_http_client()?;
    let release_url = env_trimmed(SOROTTE_GITHUB_RELEASE_LATEST_URL_ENV)
        .unwrap_or_else(|| GITHUB_RELEASE_LATEST_URL.to_owned());
    let release: GitHubRelease = match github_get_json(&client, &release_url) {
        Ok(release) => release,
        Err(error) if error.contains("HTTP 404") => {
            return Ok(LegacyUpdateCheckResult {
                status: LegacyUpdateCheckStatus::UpToDate,
                message: github_update_up_to_date_message(language, UpdateChannel::Stable),
                url: Some(GITHUB_RELEASES_PAGE_URL.to_owned()),
                candidate: None,
                self_update_supported: self_update_supported_current_install(),
                public_servers: None,
                checked_at_utc: String::new(),
                user_initiated: false,
            });
        }
        Err(error) => return Err(error),
    };
    let (manifest, package_download_url) =
        release_manifest_and_package_url(&client, &release, UpdateChannel::Stable)?;

    let current_version = current_semver()?;
    let candidate_version = parse_version(&manifest.version)?;
    if candidate_version <= current_version {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: github_update_up_to_date_message(language, UpdateChannel::Stable),
            url: release.html_url,
            candidate: None,
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        });
    }

    let candidate = candidate_from_manifest(
        manifest,
        package_download_url,
        release.html_url,
        UpdateCandidateSource::ReleaseAsset,
    );
    Ok(LegacyUpdateCheckResult {
        status: LegacyUpdateCheckStatus::UpdateAvailable,
        message: candidate.summary(),
        url: candidate.details_url.clone(),
        candidate: Some(candidate),
        self_update_supported: self_update_supported_current_install(),
        public_servers: None,
        checked_at_utc: String::new(),
        user_initiated: false,
    })
}

fn check_dev_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    if env_trimmed(SOROTTE_GITHUB_ARTIFACTS_URL_ENV).is_some() {
        check_dev_artifact_update(language)
    } else {
        check_dev_release_update(language)
    }
}

fn check_dev_release_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    let client = github_http_client()?;
    let release_url = env_trimmed(SOROTTE_GITHUB_DEV_RELEASE_URL_ENV)
        .unwrap_or_else(|| GITHUB_DEV_RELEASE_URL.to_owned());
    let release: GitHubRelease = github_get_json(&client, &release_url)?;
    let (manifest, package_download_url) =
        release_manifest_and_package_url(&client, &release, UpdateChannel::Dev)?;

    if !dev_manifest_newer_than_current(&manifest) {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: github_update_up_to_date_message(language, UpdateChannel::Dev),
            url: release.html_url,
            candidate: None,
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        });
    }

    let candidate = candidate_from_manifest(
        manifest,
        package_download_url,
        release.html_url,
        UpdateCandidateSource::ReleaseAsset,
    );
    Ok(LegacyUpdateCheckResult {
        status: LegacyUpdateCheckStatus::UpdateAvailable,
        message: candidate.summary(),
        url: candidate.details_url.clone(),
        candidate: Some(candidate),
        self_update_supported: self_update_supported_current_install(),
        public_servers: None,
        checked_at_utc: String::new(),
        user_initiated: false,
    })
}

fn check_dev_artifact_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    let client = github_http_client()?;
    let artifacts_url = env_trimmed(SOROTTE_GITHUB_ARTIFACTS_URL_ENV)
        .ok_or_else(|| format!("{SOROTTE_GITHUB_ARTIFACTS_URL_ENV} must be set"))?;
    let response: GitHubArtifactsResponse = github_get_json(&client, &artifacts_url)?;
    let Some(artifact) = select_newest_dev_artifact(&response.artifacts) else {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: github_update_up_to_date_message(language, UpdateChannel::Dev),
            url: Some("https://github.com/ropbet-radbyt/sorotte/actions".to_owned()),
            candidate: None,
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        });
    };

    if !dev_artifact_newer_than_current(artifact) {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: github_update_up_to_date_message(language, UpdateChannel::Dev),
            url: artifact
                .workflow_run
                .as_ref()
                .and_then(|run| run.html_url.clone()),
            candidate: None,
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        });
    }

    let candidate = UpdateCandidate {
        channel: UpdateChannel::Dev,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: artifact
            .workflow_run
            .as_ref()
            .and_then(|run| run.head_sha.clone()),
        created_at_utc: artifact.created_at.clone(),
        target: SOROTTE_GUI_TARGET.to_owned(),
        package: artifact.name.clone(),
        sha256: String::new(),
        download_url: artifact.archive_download_url.clone(),
        details_url: artifact
            .workflow_run
            .as_ref()
            .and_then(|run| run.html_url.clone()),
        source: UpdateCandidateSource::ActionsArtifact,
    };
    Ok(LegacyUpdateCheckResult {
        status: LegacyUpdateCheckStatus::UpdateAvailable,
        message: candidate.summary(),
        url: candidate.details_url.clone(),
        candidate: Some(candidate),
        self_update_supported: self_update_supported_current_install(),
        public_servers: None,
        checked_at_utc: String::new(),
        user_initiated: false,
    })
}

pub(crate) fn should_run_automatic_update_check(
    settings: Option<&StoredClientSettingsMvp>,
    now: std::time::SystemTime,
) -> bool {
    let Some(settings) = settings else {
        return false;
    };
    automatic_update_check_due(
        settings.check_for_updates_automatically == Some(true),
        settings.last_checked_for_updates.as_deref(),
        now,
    )
}

pub(crate) fn automatic_update_check_due(
    automatic: bool,
    last_checked_for_updates: Option<&str>,
    now: std::time::SystemTime,
) -> bool {
    if !automatic {
        return false;
    }
    let Some(last_checked) =
        last_checked_for_updates.and_then(parse_legacy_utc_timestamp_legacy_compatible)
    else {
        return true;
    };

    now.duration_since(last_checked)
        .map(|elapsed| elapsed.as_secs() > LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS)
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    html_url: Option<String>,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubArtifactsResponse {
    artifacts: Vec<GitHubArtifact>,
}

#[derive(Debug, Deserialize)]
struct GitHubArtifact {
    name: String,
    archive_download_url: String,
    expired: bool,
    created_at: String,
    #[serde(default)]
    workflow_run: Option<GitHubWorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct GitHubWorkflowRun {
    #[serde(default)]
    head_sha: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

fn github_http_client() -> Result<Client, String> {
    http_client().map_err(|error| format!("failed to build GitHub update HTTP client: {error}"))
}

fn github_get_json<T>(client: &Client, url: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let mut request = client
        .get(url)
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = env_trimmed(SOROTTE_GUI_GITHUB_TOKEN_ENV) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .map_err(|error| format!("failed to request GitHub update metadata: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to request GitHub update metadata: HTTP {}",
            response.status()
        ));
    }
    let body = response
        .text()
        .map_err(|error| format!("failed to read GitHub update metadata: {error}"))?;
    serde_json::from_str::<T>(&body)
        .map_err(|error| format!("failed to parse GitHub update metadata: {error}"))
}

fn github_download_bytes(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let mut request = client.get(url).header("Accept", "application/octet-stream");
    if let Some(token) = env_trimmed(SOROTTE_GUI_GITHUB_TOKEN_ENV) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .map_err(|error| format!("failed to download update package: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to download update package: HTTP {}",
            response.status()
        ));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("failed to read update package bytes: {error}"))
}

fn release_manifest_and_package_url(
    client: &Client,
    release: &GitHubRelease,
    expected_channel: UpdateChannel,
) -> Result<(UpdateManifest, String), String> {
    let manifest_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == SOROTTE_UPDATE_MANIFEST_NAME)
        .ok_or_else(|| "GitHub Release does not include a GUI update manifest".to_owned())?;
    let manifest: UpdateManifest = github_get_json(client, &manifest_asset.browser_download_url)?;
    validate_manifest(&manifest, expected_channel)?;
    let package_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == manifest.package)
        .or_else(|| select_stable_gui_release_asset(&release.assets, &manifest.version))
        .ok_or_else(|| {
            format!(
                "GitHub Release does not include expected GUI package {}",
                manifest.package
            )
        })?;
    Ok((manifest, package_asset.browser_download_url.clone()))
}

fn candidate_from_manifest(
    manifest: UpdateManifest,
    download_url: String,
    details_url: Option<String>,
    source: UpdateCandidateSource,
) -> UpdateCandidate {
    UpdateCandidate {
        channel: manifest.channel,
        version: manifest.version,
        git_sha: manifest.git_sha,
        created_at_utc: manifest.created_at_utc,
        target: manifest.target,
        package: manifest.package,
        sha256: manifest.sha256,
        download_url,
        details_url,
        source,
    }
}

fn validate_manifest(
    manifest: &UpdateManifest,
    expected_channel: UpdateChannel,
) -> Result<(), String> {
    if manifest.schema.trim() != "sorotte-gui-update-manifest-v1" {
        return Err(format!(
            "unsupported update manifest schema {:?}",
            manifest.schema
        ));
    }
    if manifest.app.trim() != SOROTTE_GUI_APP_NAME {
        return Err(format!("update manifest is for app {:?}", manifest.app));
    }
    if manifest.channel != expected_channel {
        return Err(format!(
            "update manifest channel {} does not match requested {} channel",
            manifest.channel.label(),
            expected_channel.label()
        ));
    }
    if manifest.target.trim() != SOROTTE_GUI_TARGET {
        return Err(format!(
            "update manifest target {} does not match {}",
            manifest.target, SOROTTE_GUI_TARGET
        ));
    }
    if !normal_package_basename(&manifest.package)
        || !manifest.package.starts_with("sorotte-gui-")
        || !manifest
            .package
            .ends_with(SOROTTE_GUI_RELEASE_PACKAGE_SUFFIX)
    {
        return Err(format!(
            "update manifest package has unexpected name {}",
            manifest.package
        ));
    }
    validate_sha256_hex(&manifest.sha256)?;
    Ok(())
}

fn normal_package_basename(value: &str) -> bool {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.contains(['/', '\\', ':'])
        || has_windows_drive_prefix(value)
    {
        return false;
    }
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn validate_sha256_hex(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("update manifest SHA-256 must be 64 hexadecimal characters".to_owned())
    }
}

fn select_stable_gui_release_asset<'a>(
    assets: &'a [GitHubReleaseAsset],
    version: &str,
) -> Option<&'a GitHubReleaseAsset> {
    let normalized_version = version.trim().trim_start_matches('v');
    let expected = format!("sorotte-gui-{normalized_version}{SOROTTE_GUI_RELEASE_PACKAGE_SUFFIX}");
    assets.iter().find(|asset| asset.name == expected)
}

fn select_newest_dev_artifact(artifacts: &[GitHubArtifact]) -> Option<&GitHubArtifact> {
    artifacts
        .iter()
        .filter(|artifact| artifact.name == SOROTTE_GUI_DEV_ARTIFACT_NAME)
        .filter(|artifact| !artifact.expired)
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
}

fn dev_artifact_newer_than_current(artifact: &GitHubArtifact) -> bool {
    dev_candidate_newer_than_current(
        artifact
            .workflow_run
            .as_ref()
            .and_then(|run| run.head_sha.as_deref()),
        &artifact.created_at,
    )
}

fn dev_manifest_newer_than_current(manifest: &UpdateManifest) -> bool {
    dev_candidate_newer_than_current(manifest.git_sha.as_deref(), &manifest.created_at_utc)
}

fn dev_candidate_newer_than_current(
    candidate_sha: Option<&str>,
    candidate_created_at: &str,
) -> bool {
    let install_marker = current_install_marker();
    let current_sha = env_trimmed(SOROTTE_GUI_BUILD_GIT_SHA_ENV).or_else(|| {
        install_marker
            .as_ref()
            .and_then(|marker| marker.git_sha.clone())
    });
    let current_created_at = env_trimmed(SOROTTE_GUI_BUILD_CREATED_AT_UTC_ENV).or_else(|| {
        install_marker
            .as_ref()
            .and_then(|marker| marker.created_at_utc.clone())
    });
    dev_candidate_is_newer(
        candidate_sha,
        candidate_created_at,
        current_sha.as_deref(),
        current_created_at.as_deref(),
    )
}

fn dev_candidate_is_newer(
    candidate_sha: Option<&str>,
    candidate_created_at: &str,
    current_sha: Option<&str>,
    current_created_at: Option<&str>,
) -> bool {
    if let (Some(candidate_sha), Some(current_sha)) = (candidate_sha, current_sha) {
        // The rolling dev release is published only from the verified current
        // main tip. Git author/committer timestamps are not monotonic along
        // history, so a different authoritative SHA is the newer build even
        // when its timestamp sorts earlier.
        return !candidate_sha
            .trim()
            .eq_ignore_ascii_case(current_sha.trim());
    }
    match current_created_at {
        Some(current_created_at) => candidate_created_at > current_created_at,
        None => true,
    }
}

fn current_semver() -> Result<Version, String> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

fn parse_version(value: &str) -> Result<Version, String> {
    Version::parse(value.trim().trim_start_matches('v'))
        .map_err(|error| format!("failed to parse update version {value:?}: {error}"))
}

fn update_supported_platform() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelfUpdateCapability {
    SupportedWritable,
    RequiresElevation,
    TargetDirectoryNotWritable,
    UnsupportedPlatform,
    UnpackagedInstall,
}

impl SelfUpdateCapability {
    fn supported(self) -> bool {
        self == Self::SupportedWritable
    }

    fn unavailable_message(self) -> &'static str {
        match self {
            Self::SupportedWritable => "Automatic self-update is available.",
            Self::RequiresElevation => {
                "This Sorotte GUI install requires elevation to update. Automatic elevation is disabled until releases have a pinned signing trust anchor; install the release manually."
            }
            Self::TargetDirectoryNotWritable => {
                "This Sorotte GUI install directory is not writable, so automatic self-update is unavailable. Install the release manually."
            }
            Self::UnsupportedPlatform => {
                "GitHub self-update is currently supported for Windows x64 GUI packages only."
            }
            Self::UnpackagedInstall => {
                "GitHub self-update is only available for packaged Sorotte GUI installs."
            }
        }
    }
}

fn current_install_marker_path() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join(SOROTTE_GUI_INSTALL_MARKER))
    })
}

fn current_install_marker() -> Option<GuiInstallMarker> {
    let path = current_install_marker_path()?;
    let body = fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn self_update_supported_current_install() -> bool {
    self_update_capability_current_install().supported()
}

fn self_update_capability_current_install() -> SelfUpdateCapability {
    let Ok(current_exe) = std::env::current_exe() else {
        return SelfUpdateCapability::UnpackagedInstall;
    };
    #[cfg(windows)]
    let requires_elevation = current_exe
        .parent()
        .is_some_and(path_requires_update_elevation);
    #[cfg(not(windows))]
    let requires_elevation = false;
    self_update_capability_for_install(
        &current_exe,
        update_supported_platform(),
        requires_elevation,
    )
}

fn self_update_capability_for_install(
    current_exe: &Path,
    platform_supported: bool,
    requires_elevation: bool,
) -> SelfUpdateCapability {
    self_update_capability_for_install_with_probe(
        current_exe,
        platform_supported,
        requires_elevation,
        update_target_directory_is_writable,
    )
}

fn self_update_capability_for_install_with_probe<F>(
    current_exe: &Path,
    platform_supported: bool,
    requires_elevation: bool,
    writable_probe: F,
) -> SelfUpdateCapability
where
    F: FnOnce(&Path) -> bool,
{
    if !platform_supported {
        return SelfUpdateCapability::UnsupportedPlatform;
    }
    let Some(target_dir) = current_exe.parent() else {
        return SelfUpdateCapability::UnpackagedInstall;
    };
    if !target_dir.join(SOROTTE_GUI_INSTALL_MARKER).is_file() {
        return SelfUpdateCapability::UnpackagedInstall;
    }
    if requires_elevation {
        return SelfUpdateCapability::RequiresElevation;
    }
    if !writable_probe(target_dir) {
        return SelfUpdateCapability::TargetDirectoryNotWritable;
    }
    SelfUpdateCapability::SupportedWritable
}

fn update_target_directory_is_writable(target_dir: &Path) -> bool {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let probe = target_dir.join(format!(
        ".sorotte-update-write-probe-{}-{nonce}",
        std::process::id(),
    ));
    let directory_probe_succeeded = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            fs::remove_file(probe).is_ok()
        }
        Err(_) => false,
    };
    directory_probe_succeeded
        && [
            SOROTTE_GUI_EXECUTABLE,
            SOROTTE_GUI_UPDATER_EXECUTABLE,
            SOROTTE_GUI_INSTALL_MARKER,
        ]
        .iter()
        .all(|name| update_target_file_is_replaceable(&target_dir.join(name)))
}

#[cfg(windows)]
fn update_target_file_is_replaceable(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    const DELETE_ACCESS: u32 = 0x0001_0000;

    fs::OpenOptions::new()
        .access_mode(DELETE_ACCESS)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .is_ok()
}

#[cfg(not(windows))]
fn update_target_file_is_replaceable(path: &Path) -> bool {
    fs::OpenOptions::new().write(true).open(path).is_ok()
}

fn github_update_response_override_result(
    body: &str,
    language: Option<&str>,
) -> Result<Option<LegacyUpdateCheckResult>, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Ok(manifest) = serde_json::from_str::<UpdateManifest>(trimmed) {
        validate_manifest(&manifest, manifest.channel)?;
        let update_available = match manifest.channel {
            UpdateChannel::Stable => parse_version(&manifest.version)? > current_semver()?,
            UpdateChannel::Dev => dev_manifest_newer_than_current(&manifest),
        };
        if !update_available {
            return Ok(Some(LegacyUpdateCheckResult {
                status: LegacyUpdateCheckStatus::UpToDate,
                message: github_update_up_to_date_message(language, manifest.channel),
                url: None,
                candidate: None,
                self_update_supported: self_update_supported_current_install(),
                public_servers: None,
                checked_at_utc: String::new(),
                user_initiated: false,
            }));
        }
        let candidate = candidate_from_manifest(
            manifest,
            "https://example.invalid/sorotte-gui-update.zip".to_owned(),
            None,
            UpdateCandidateSource::ReleaseAsset,
        );
        return Ok(Some(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpdateAvailable,
            message: candidate.summary(),
            url: None,
            candidate: Some(candidate),
            self_update_supported: self_update_supported_current_install(),
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        }));
    }
    Ok(None)
}

fn github_update_up_to_date_message(language: Option<&str>, channel: UpdateChannel) -> String {
    let base = localized_literal(
        language,
        "Sorotte is up to date",
        "Sorotte ist auf dem neuesten Stand",
        "Sorotte esta actualizado",
        "Sorotte estas gxisdata",
        "Sorotte on ajan tasalla",
        "Sorotte est a jour",
        "Sorotte e aggiornato",
        "O Sorotte esta atualizado",
        "Sorotte guncel",
        "Sorotte obnovlen do poslednei versii",
        "Sorotte yi shi zuixin banben",
        "Sorotteneun choesin sangtaeimnida",
    );
    match channel {
        UpdateChannel::Stable => base.to_owned(),
        UpdateChannel::Dev => format!("{base} on the dev channel"),
    }
}

fn github_update_check_failed_message(language: Option<&str>) -> String {
    localized_literal(
        language,
        "Could not check GitHub for Sorotte GUI updates. Open the GitHub releases page to check manually.",
        "GitHub konnte nicht auf Sorotte-GUI-Updates geprueft werden. Oeffnen Sie die GitHub-Releases-Seite zur manuellen Pruefung.",
        "No se pudo comprobar GitHub para actualizaciones de Sorotte GUI. Abra la pagina de lanzamientos de GitHub para comprobar manualmente.",
        "Ne eblis kontroli GitHub por Sorotte GUI gxisdatigoj. Malfermu la GitHub eldonpaghon por kontroli mane.",
        "GitHubista ei voitu tarkistaa Sorotte GUI -paivityksia. Tarkista paivitykset kasin GitHub-julkaisusivulta.",
        "Impossible de verifier les mises a jour de l'interface Sorotte sur GitHub. Ouvrez la page des versions GitHub pour verifier manuellement.",
        "Impossibile controllare GitHub per aggiornamenti della GUI di Sorotte. Apri la pagina release di GitHub per controllare manualmente.",
        "Nao foi possivel verificar atualizacoes da GUI do Sorotte no GitHub. Abra a pagina de lancamentos do GitHub para verificar manualmente.",
        "GitHub'da Sorotte GUI guncellemeleri denetlenemedi. Elle denetlemek icin GitHub surumleri sayfasini acin.",
        "Ne udalos proverit GitHub na obnovleniia Sorotte GUI. Otkroite stranicu vypuskov GitHub dlia ruchnoi proverki.",
        "Wu fa zai GitHub shang jiancha Sorotte GUI gengxin. Qing dakai GitHub fabu yemian shoudong jiancha.",
        "GitHub-eseo Sorotte GUI eobdeiteureul hwaginhal su eopseotseumnida. susdong hwagineul wihae GitHub baepo peijireul yeoreojuseyo.",
    )
    .to_owned()
}

fn download_and_stage_update_inner(
    candidate: &UpdateCandidate,
    gui_config_root: Option<&Path>,
) -> Result<StagedUpdate, String> {
    let capability = self_update_capability_current_install();
    if !capability.supported() {
        return Err(capability.unavailable_message().to_owned());
    }
    let Some(gui_config_root) = gui_config_root else {
        return Err(
            "Cannot stage update because the Sorotte GUI config root is unavailable.".to_owned(),
        );
    };
    let client = github_http_client()?;
    let updates_root = gui_config_root.join("updates");
    fs::create_dir_all(&updates_root).map_err(|error| {
        format!(
            "failed to create update staging directory {}: {error}",
            updates_root.display()
        )
    })?;
    let stage_dir = updates_root.join(format!(
        "{}-{}",
        candidate.channel.label(),
        sanitize_stage_name(&candidate.created_at_utc)
    ));
    cleanup_updates_root(&updates_root, &stage_dir)?;
    if stage_dir.exists() {
        fs::remove_dir_all(&stage_dir).map_err(|error| {
            format!(
                "failed to clear previous staged update {}: {error}",
                stage_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&stage_dir).map_err(|error| {
        format!(
            "failed to create staged update directory {}: {error}",
            stage_dir.display()
        )
    })?;

    stage_update_payload(candidate, &client, &stage_dir)
        .map_err(|error| cleanup_failed_stage_dir(&stage_dir, error))
}

fn cleanup_updates_root(updates_root: &Path, active_stage_dir: &Path) -> Result<(), String> {
    let active_stage_name = active_stage_dir
        .file_name()
        .ok_or_else(|| "active update stage directory has no name".to_owned())?;
    cleanup_updates_root_entries(updates_root, Some(active_stage_name))
}

fn cleanup_updates_root_entries(
    updates_root: &Path,
    active_stage_name: Option<&std::ffi::OsStr>,
) -> Result<(), String> {
    let root_metadata = fs::symlink_metadata(updates_root).map_err(|error| {
        format!(
            "failed to inspect update staging directory {}: {error}",
            updates_root.display()
        )
    })?;
    if metadata_is_link_or_reparse(&root_metadata) || !root_metadata.is_dir() {
        return Err(format!(
            "update staging path is not a regular directory: {}",
            updates_root.display()
        ));
    }
    for entry in fs::read_dir(updates_root).map_err(|error| {
        format!(
            "failed to read update staging directory {}: {error}",
            updates_root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read update staging directory entry in {}: {error}",
                updates_root.display()
            )
        })?;
        let entry_name = entry.file_name();
        if active_stage_name.is_some_and(|active_stage_name| entry_name == active_stage_name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect stale update staging entry {}: {error}",
                path.display()
            )
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(format!(
                "refusing to remove stale update staging link or reparse point: {}",
                path.display()
            ));
        }
        remove_update_staging_entry(&path, metadata.file_type())?;
    }
    Ok(())
}

fn remove_update_staging_entry(path: &Path, file_type: fs::FileType) -> Result<(), String> {
    let result = if file_type.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove stale update staging entry {}: {error}",
            path.display()
        )),
    }
}

fn cleanup_failed_stage_dir(stage_dir: &Path, error: String) -> String {
    match fs::remove_dir_all(stage_dir) {
        Ok(()) => error,
        Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => error,
        Err(cleanup_error) => format!(
            "{error}; additionally failed to clean partial staged update {}: {cleanup_error}",
            stage_dir.display()
        ),
    }
}

fn stage_update_payload(
    candidate: &UpdateCandidate,
    client: &Client,
    stage_dir: &Path,
) -> Result<StagedUpdate, String> {
    let downloaded_bytes = github_download_bytes(client, &candidate.download_url)?;
    let (package_bytes, staged_candidate) = match candidate.source {
        UpdateCandidateSource::ReleaseAsset => {
            validate_sha256_bytes(&downloaded_bytes, &candidate.sha256)?;
            (downloaded_bytes, candidate.clone())
        }
        UpdateCandidateSource::ActionsArtifact => {
            let artifact_dir = stage_dir.join("artifact");
            extract_zip_bytes_safe(&downloaded_bytes, &artifact_dir)?;
            let manifest_path = artifact_dir.join(SOROTTE_UPDATE_MANIFEST_NAME);
            let manifest = read_manifest_file(&manifest_path)?;
            validate_manifest(&manifest, UpdateChannel::Dev)?;
            let package_path = artifact_dir.join(&manifest.package);
            let package_bytes = fs::read(&package_path).map_err(|error| {
                format!(
                    "failed to read update package from artifact {}: {error}",
                    package_path.display()
                )
            })?;
            validate_sha256_bytes(&package_bytes, &manifest.sha256)?;
            (
                package_bytes,
                candidate_from_manifest(
                    manifest,
                    candidate.download_url.clone(),
                    candidate.details_url.clone(),
                    candidate.source,
                ),
            )
        }
    };
    validate_candidate_target(&staged_candidate)?;
    let package_path = stage_dir.join(&staged_candidate.package);
    fs::write(&package_path, &package_bytes).map_err(|error| {
        format!(
            "failed to write staged update package {}: {error}",
            package_path.display()
        )
    })?;
    let source_dir = stage_dir.join("extracted");
    extract_zip_bytes_safe(&package_bytes, &source_dir)?;
    validate_extracted_update_payload(&source_dir)?;

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current GUI executable: {error}"))?;
    let backup_dir = stage_dir.join("backup");
    fs::create_dir_all(&backup_dir).map_err(|error| {
        format!(
            "failed to create update backup directory {}: {error}",
            backup_dir.display()
        )
    })?;
    let updater_path = current_exe
        .parent()
        .ok_or_else(|| "current GUI executable has no parent directory".to_owned())?
        .join(SOROTTE_GUI_UPDATER_EXECUTABLE);
    if !updater_path.is_file() {
        return Err(format!(
            "installed update helper is missing: {}",
            updater_path.display()
        ));
    }
    let log_path = stage_dir.join("sorotte-gui-updater.log");
    Ok(StagedUpdate {
        candidate: staged_candidate,
        package_path: package_path.display().to_string(),
        source_dir: source_dir.display().to_string(),
        updater_path: updater_path.display().to_string(),
        target_exe_path: current_exe.display().to_string(),
        backup_dir: backup_dir.display().to_string(),
        log_path: log_path.display().to_string(),
        restart: true,
    })
}

fn validate_candidate_target(candidate: &UpdateCandidate) -> Result<(), String> {
    if candidate.target != SOROTTE_GUI_TARGET {
        return Err(format!(
            "update package targets {}, but this build expects {}",
            candidate.target, SOROTTE_GUI_TARGET
        ));
    }
    validate_sha256_hex(&candidate.sha256)
}

fn read_manifest_file(path: &Path) -> Result<UpdateManifest, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read update manifest {}: {error}", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| {
        format!(
            "failed to parse update manifest {}: {error}",
            path.display()
        )
    })
}

fn validate_sha256_bytes(bytes: &[u8], expected: &str) -> Result<(), String> {
    validate_sha256_hex(expected)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = lowercase_hex(hasher.finalize());
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(format!(
            "update package SHA-256 mismatch: expected {}, got {}",
            expected.trim(),
            actual
        ))
    }
}

fn extract_zip_bytes_safe(bytes: &[u8], destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create zip extraction directory {}: {error}",
            destination.display()
        )
    })?;
    let reader = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(reader).map_err(|error| format!("failed to open update zip: {error}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to read update zip entry {index}: {error}"))?;
        let Some(relative_path) = safe_zip_relative_path(entry.name()) else {
            return Err(format!(
                "update zip contains unsafe path {:?}",
                entry.name()
            ));
        };
        let output_path = destination.join(relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                format!(
                    "failed to create update zip directory {}: {error}",
                    output_path.display()
                )
            })?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create update zip parent directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut output = fs::File::create(&output_path).map_err(|error| {
            format!(
                "failed to create update zip output {}: {error}",
                output_path.display()
            )
        })?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            format!(
                "failed to extract update zip entry {}: {error}",
                output_path.display()
            )
        })?;
    }
    Ok(())
}

fn safe_zip_relative_path(name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || has_windows_drive_prefix(name)
    {
        return None;
    }
    let mut safe = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            _ if part.contains(':') => return None,
            _ => {
                let path = Path::new(part);
                let mut components = path.components();
                if !matches!(components.next(), Some(Component::Normal(_)))
                    || components.next().is_some()
                {
                    return None;
                }
                safe.push(part);
            }
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn has_windows_drive_prefix(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_extracted_update_payload(source_dir: &Path) -> Result<(), String> {
    for required in [SOROTTE_GUI_EXECUTABLE, SOROTTE_GUI_UPDATER_EXECUTABLE] {
        let path = source_dir.join(required);
        if !path.is_file() {
            return Err(format!(
                "staged update is missing required file {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn launch_staged_update_inner(staged_update: &StagedUpdate) -> Result<(), String> {
    let capability = self_update_capability_current_install();
    if !capability.supported() {
        return Err(capability.unavailable_message().to_owned());
    }
    let current_pid = std::process::id().to_string();
    let target_exe = PathBuf::from(&staged_update.target_exe_path);
    let target_dir = target_exe
        .parent()
        .ok_or_else(|| "current GUI executable has no parent directory".to_owned())?;
    let helper_args = staged_update_helper_args(staged_update, target_dir, &current_pid);
    let mut command = Command::new(&staged_update.updater_path);
    command.args(&helper_args);
    configure_gui_child_process(&mut command);
    command
        .spawn()
        .map_err(|error| format!("failed to launch update helper: {error}"))?;
    Ok(())
}

fn staged_update_helper_args(
    staged_update: &StagedUpdate,
    target_dir: &Path,
    current_pid: &str,
) -> Vec<String> {
    let mut args = vec![
        "--pid".to_owned(),
        current_pid.to_owned(),
        "--package".to_owned(),
        staged_update.package_path.clone(),
        "--package-sha256".to_owned(),
        staged_update.candidate.sha256.clone(),
        "--target-dir".to_owned(),
        target_dir.display().to_string(),
        "--target-exe".to_owned(),
        SOROTTE_GUI_EXECUTABLE.to_owned(),
        "--log".to_owned(),
        staged_update.log_path.clone(),
    ];
    if staged_update.restart {
        args.push("--restart".to_owned());
    }
    args
}

#[cfg(windows)]
fn path_requires_update_elevation(path: &Path) -> bool {
    program_files_roots()
        .iter()
        .any(|root| path_is_equal_or_child_case_insensitive(path, root))
}

#[cfg(windows)]
fn program_files_roots() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(windows)]
fn path_is_equal_or_child_case_insensitive(path: &Path, root: &Path) -> bool {
    let path = normalized_windows_path_text(path);
    let root = normalized_windows_path_text(root);
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

#[cfg(windows)]
fn normalized_windows_path_text(path: &Path) -> String {
    let mut text = fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
        .replace('/', "\\");
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_owned();
    }
    text.trim_end_matches('\\').to_ascii_lowercase()
}

fn sanitize_stage_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.trim_matches('-').is_empty() {
        "update".to_owned()
    } else {
        sanitized
    }
}

fn short_git_sha(value: &str) -> &str {
    value.get(..7).unwrap_or(value)
}

fn env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
fn fetch_update_check_result_from_url(
    url: &str,
    language: Option<&str>,
    user_initiated: bool,
) -> Result<LegacyUpdateCheckResult, String> {
    let language = normalized_language(language);
    let client = http_client()
        .map_err(|error| format!("failed to build update-check HTTP client: {error}"))?;
    let response = client
        .get(url)
        .query(&[
            ("version", LEGACY_SYNCPLAY_VERSION),
            ("milestone", LEGACY_SYNCPLAY_MILESTONE),
            ("release_number", LEGACY_SYNCPLAY_RELEASE_NUMBER),
            ("language", language),
            ("platform", legacy_update_check_platform_name()),
            ("architecture", std::env::consts::ARCH),
            ("machine", std::env::consts::ARCH),
            (
                "userInitiated",
                if user_initiated { "True" } else { "False" },
            ),
        ])
        .send()
        .map_err(|error| format!("failed to run update check: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "failed to run update check: HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|error| format!("failed to read update-check response: {error}"))?;
    parse_update_check_response(&body, Some(language), user_initiated)
}

#[cfg(test)]
fn parse_update_check_response(
    body: &str,
    language: Option<&str>,
    user_initiated: bool,
) -> Result<LegacyUpdateCheckResult, String> {
    let normalized = sanitize_wordpress_update_check_response(body);
    let parsed = serde_json::from_str::<Value>(&normalized)
        .map_err(|error| format!("failed to parse update-check response: {error}"))?;
    let raw_status = parsed
        .get("version-status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let status = LegacyUpdateCheckStatus::from_legacy_wire_value(raw_status);
    let message = parsed
        .get("version-message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|message| localize_wire_update_message(message, language))
        .unwrap_or_else(|| default_update_check_message(&status, language));
    let mut url = parsed
        .get("version-url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if url.is_none()
        && matches!(
            status,
            LegacyUpdateCheckStatus::Failed | LegacyUpdateCheckStatus::Unknown(_)
        )
        && user_initiated
    {
        url = Some(SYNCPLAY_DOWNLOAD_URL.to_owned());
    }
    let public_servers = parsed
        .get("public-servers")
        .and_then(Value::as_str)
        .map(parse_public_server_response)
        .transpose()?;

    Ok(LegacyUpdateCheckResult {
        status,
        message,
        url,
        candidate: None,
        self_update_supported: false,
        public_servers,
        checked_at_utc: String::new(),
        user_initiated,
    })
}

fn http_client() -> Result<Client, reqwest::Error> {
    ensure_rustls_crypto_provider();
    Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("sorotte-gui/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

fn env_response_override(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn ensure_rustls_crypto_provider() {
    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn sanitize_wordpress_public_server_response(body: &str) -> String {
    body.replace("<p>", "")
        .replace("</p>", "")
        .replace("<br />", "")
        .replace("&#8220;", "'")
        .replace("&#8221;", "'")
        .replace(":&#8217;", "'")
        .replace("&#8217;", "'")
        .replace("&#8242;", "'")
        .replace(['\n', '\r'], "")
}

#[cfg(test)]
fn sanitize_wordpress_update_check_response(body: &str) -> String {
    body.replace("<p>", "")
        .replace("</p>", "")
        .replace("<br />", "")
        .replace("&#8220;", "\"")
        .replace("&#8221;", "\"")
        .replace(['\n', '\r'], "")
}

#[cfg(test)]
fn legacy_update_check_platform_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn normalized_language(language: Option<&str>) -> &'static str {
    language
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .unwrap_or("en")
}

#[allow(clippy::too_many_arguments)]
fn localized_literal(
    language: Option<&str>,
    en: &'static str,
    de: &'static str,
    es: &'static str,
    eo: &'static str,
    fi: &'static str,
    fr: &'static str,
    it: &'static str,
    pt: &'static str,
    tr: &'static str,
    ru: &'static str,
    zh_cn: &'static str,
    ko: &'static str,
) -> &'static str {
    match normalized_language(language) {
        "de" => de,
        "es" => es,
        "eo" => eo,
        "fi" => fi,
        "fr" => fr,
        "it" => it,
        "pt_PT" | "pt_BR" => pt,
        "tr" => tr,
        "ru" => ru,
        "zh_CN" => zh_cn,
        "ko" => ko,
        _ => en,
    }
}

fn public_server_list_failed_message(language: Option<&str>) -> &'static str {
    localized_literal(
        language,
        "Failed to load public server list. Please visit https://www.syncplay.pl/ in your browser.",
        "Die Liste der oeffentlichen Server konnte nicht geladen werden. Bitte besuchen Sie https://www.syncplay.pl/ in Ihrem Browser.",
        "No se pudo cargar la lista de servidores publicos. Visite https://www.syncplay.pl/ en su navegador.",
        "Malsukcesis sxargi la liston de publikaj serviloj. Bonvolu viziti https://www.syncplay.pl/ en via retumilo.",
        "Julkisten palvelinten listaa ei voitu ladata. Kay osoitteessa https://www.syncplay.pl/ selaimessasi.",
        "Echec du chargement de la liste des serveurs publics. Veuillez visiter https://www.syncplay.pl/ dans votre navigateur.",
        "Impossibile caricare l'elenco dei server pubblici. Visita https://www.syncplay.pl/ nel browser.",
        "Falha ao carregar a lista de servidores publicos. Visite https://www.syncplay.pl/ no navegador.",
        "Genel sunucu listesi yuklenemedi. Lutfen tarayicinizda https://www.syncplay.pl/ adresini ziyaret edin.",
        "Ne udalos zagruzit spisok publichnykh serverov. Pozhaluista, otkroite https://www.syncplay.pl/ v brauzere.",
        "Wu fa jiazai gonggong fuwuqi liebiao. Qing zai liulanqi zhong fangwen https://www.syncplay.pl/ .",
        "gonggae seobeo mongnog-eul bulleo-oji moshaetseumnida. beuraujeoeseo https://www.syncplay.pl/ reul yeoreojuseyo.",
    )
}

#[cfg(test)]
fn default_update_check_message(
    status: &LegacyUpdateCheckStatus,
    language: Option<&str>,
) -> String {
    match status {
        LegacyUpdateCheckStatus::UpToDate => localized_literal(
            language,
            "Sorotte is up to date",
            "Sorotte ist auf dem neuesten Stand",
            "Sorotte esta actualizado",
            "Sorotte estas gxisdata",
            "Sorotte on ajan tasalla",
            "Sorotte est a jour",
            "Sorotte e aggiornato",
            "O Sorotte esta atualizado",
            "Sorotte guncel",
            "Sorotte obnovlen do poslednei versii",
            "Sorotte yi shi zuixin banben",
            "Sorotteneun choesin sangtaeimnida",
        )
        .to_owned(),
        LegacyUpdateCheckStatus::UpdateAvailable => localized_literal(
            language,
            "A new version of Sorotte is available. Do you want to visit the release page?",
            "Eine neue Version von Sorotte ist verfuegbar. Moechten Sie die Release-Seite besuchen?",
            "Hay una nueva version de Sorotte disponible. Desea visitar la pagina de lanzamiento?",
            "Nova versio de Sorotte disponeblas. Chu vi volas viziti la eldonan paghon?",
            "Uusi Sorotte-versio on saatavilla. Haluatko avata julkaisusivun?",
            "Une nouvelle version de Sorotte est disponible. Voulez-vous visiter la page de publication?",
            "E disponibile una nuova versione di Sorotte. Vuoi visitare la pagina di rilascio?",
            "Uma nova versao do Sorotte esta disponivel. Deseja visitar a pagina de lancamento?",
            "Sorotte'nin yeni bir surumu mevcut. Surum sayfasini ziyaret etmek ister misiniz?",
            "Dostupna novaia versiia Sorotte. Otkryt stranicu vypuska?",
            "You xin de Sorotte banben ke yong. Yao fangwen fabu yemian ma?",
            "Sorotte-ui saeroun beojeoni isseumnida. baepo peijireul bangmunhasigesseumnikka?",
        )
        .to_owned(),
        LegacyUpdateCheckStatus::Checking
        | LegacyUpdateCheckStatus::Failed
        | LegacyUpdateCheckStatus::Unknown(_) => {
            update_check_failed_notification_message(language)
        }
    }
}

#[cfg(test)]
fn update_check_failed_notification_message(language: Option<&str>) -> String {
    localized_literal(
        language,
        "Could not automatically check whether Sorotte {} is up to date. Want to visit https://syncplay.pl/ to manually check for updates?",
        "Es konnte nicht automatisch geprueft werden, ob Sorotte {} aktuell ist. Moechten Sie https://syncplay.pl/ besuchen, um manuell nach Updates zu suchen?",
        "No se pudo comprobar automaticamente si Sorotte {} esta actualizado. Desea visitar https://syncplay.pl/ para comprobar manualmente si hay actualizaciones?",
        "Ne eblis auxtomate kontroli chu Sorotte {} estas gxisdata. Chu vi volas viziti https://syncplay.pl/ por mane kontroli gxisdatigojn?",
        "Ei voitu tarkistaa automaattisesti, onko Sorotte {} ajan tasalla. Haluatko kayda osoitteessa https://syncplay.pl/ tarkistaaksesi paivitykset manuaalisesti?",
        "Impossible de verifier automatiquement si Sorotte {} est a jour. Voulez-vous visiter https://syncplay.pl/ pour verifier manuellement les mises a jour?",
        "Impossibile verificare automaticamente se Sorotte {} e aggiornato. Vuoi visitare https://syncplay.pl/ per controllare manualmente gli aggiornamenti?",
        "Nao foi possivel verificar automaticamente se o Sorotte {} esta atualizado. Deseja visitar https://syncplay.pl/ para verificar atualizacoes manualmente?",
        "Sorotte {}'nin guncel olup olmadigi otomatik olarak denetlenemedi. Guncellemeleri elle kontrol etmek icin https://syncplay.pl/ adresini ziyaret etmek ister misiniz?",
        "Ne udalos avtomaticheski proverit, obnovlen li Sorotte {}. Hotite pereiti na https://syncplay.pl/ dlia ruchnoi proverki obnovlenii?",
        "Wu fa zidong jiancha Sorotte {} shifou wei zuixin banben. Yao fangwen https://syncplay.pl/ shoudong jiancha gengxin ma?",
        "Sorotte {}ga choesin beojeoninji jadongeuro hwaginhal su eopseotseumnida. susdong-euro eobdeiteureul hwaginhagi wihae https://syncplay.pl/ reul bangmunhasigesseumnikka?",
    )
    .replace("{}", LEGACY_SYNCPLAY_VERSION)
}

#[cfg(test)]
fn localize_wire_update_message(message: &str, language: Option<&str>) -> String {
    let trimmed = message.trim();
    match trimmed {
        "Sorotte is up to date" | "Sorotte is up to date." => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpToDate, language)
        }
        "A new version of Sorotte is available. Do you want to visit the release page?" => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpdateAvailable, language)
        }
        _ => trimmed.to_owned(),
    }
}

pub(super) fn legacy_utc_timestamp_string_legacy_compatible(now: std::time::SystemTime) -> String {
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_seconds = duration.as_secs() as i64;
    let days_since_epoch = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let millis = duration.subsec_millis();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
}

fn parse_legacy_utc_timestamp_legacy_compatible(value: &str) -> Option<std::time::SystemTime> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 23
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
    {
        return None;
    }

    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<u64>().ok()?;
    let minute = value[14..16].parse::<u64>().ok()?;
    let second = value[17..19].parse::<u64>().ok()?;
    let millis = value[20..23].parse::<u64>().ok()?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || millis > 999
    {
        return None;
    }

    let days_since_epoch =
        days_since_unix_epoch_from_civil_legacy_compatible(year, month as i64, day as i64);
    if days_since_epoch < 0 {
        return None;
    }

    let total_seconds = days_since_epoch as u64 * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(
        std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(total_seconds)
            + std::time::Duration::from_millis(millis),
    )
}

fn civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn days_since_unix_epoch_from_civil_legacy_compatible(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let yoe = adjusted_year - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        GitHubArtifact, GitHubReleaseAsset, GitHubWorkflowRun, LegacyUpdateCheckResult,
        LegacyUpdateCheckStatus, SOROTTE_GUI_EXECUTABLE, SOROTTE_GUI_INSTALL_MARKER,
        SOROTTE_GUI_TARGET, SelfUpdateCapability, StagedUpdate, StoredClientSettingsMvp,
        UpdateCandidate, UpdateCandidateSource, UpdateChannel, UpdateManifest,
        apply_self_update_capability, cleanup_failed_stage_dir, cleanup_update_staging_root,
        cleanup_updates_root, default_update_check_message, fetch_public_servers_from_url,
        fetch_update_check_result_from_url, normal_package_basename, parse_public_server_response,
        parse_update_check_response, parse_version, safe_zip_relative_path,
        sanitize_wordpress_public_server_response, sanitize_wordpress_update_check_response,
        select_newest_dev_artifact, select_stable_gui_release_asset,
        self_update_capability_for_install_with_probe, should_run_automatic_update_check,
        staged_update_helper_args, validate_manifest, validate_sha256_bytes,
    };
    #[cfg(windows)]
    use super::{path_is_equal_or_child_case_insensitive, pending_update_recovery_args};

    fn spawn_single_request_server(body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("test HTTP server should bind to localhost");
        let address = listener
            .local_addr()
            .expect("test HTTP server should expose a local address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("test HTTP server should accept a request");
            let mut buffer = [0u8; 4096];
            let bytes_read = stream
                .read(&mut buffer)
                .expect("test HTTP server should read the request");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            let request_line = request
                .lines()
                .next()
                .expect("HTTP request should contain a request line")
                .to_owned();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("test HTTP server should write the response");
            request_line
        });
        (format!("http://{address}"), handle)
    }

    fn temp_update_root(test_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-{test_name}-{}-{timestamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test update root should be created");
        root
    }

    #[test]
    fn update_stable_asset_selection_uses_versioned_windows_package() {
        let assets = vec![
            GitHubReleaseAsset {
                name: "sorotte-gui-0.2.0-linux-x86_64.zip".to_owned(),
                browser_download_url: "https://example.invalid/linux.zip".to_owned(),
            },
            GitHubReleaseAsset {
                name: "sorotte-gui-0.2.0-windows-x86_64.zip".to_owned(),
                browser_download_url: "https://example.invalid/windows.zip".to_owned(),
            },
        ];

        let selected = select_stable_gui_release_asset(&assets, "v0.2.0")
            .expect("matching Windows GUI package should be selected");

        assert_eq!(
            selected.browser_download_url,
            "https://example.invalid/windows.zip"
        );
    }

    #[test]
    fn update_default_release_urls_use_source_repo() {
        assert!(
            super::GITHUB_RELEASE_LATEST_URL
                .contains("/repos/ropbet-radbyt/sorotte/releases/latest")
        );
        assert!(
            super::GITHUB_DEV_RELEASE_URL
                .contains("/repos/ropbet-radbyt/sorotte/releases/tags/sorotte-gui-dev")
        );
        assert_eq!(
            super::GITHUB_RELEASES_PAGE_URL,
            "https://github.com/ropbet-radbyt/sorotte/releases"
        );
    }

    #[test]
    fn update_install_marker_deserializes_channel_metadata() {
        let marker: super::GuiInstallMarker = serde_json::from_str(
            r#"{
                "app": "sorotte-gui",
                "channel": "dev",
                "version": "0.1.0",
                "git_sha": "abcdef123456",
                "created_at_utc": "2026-05-21T11:17:13Z",
                "target": "windows-x86_64"
            }"#,
        )
        .expect("install marker should parse");

        assert_eq!(marker.channel, Some(UpdateChannel::Dev));
        assert_eq!(marker.git_sha.as_deref(), Some("abcdef123456"));
        assert_eq!(
            marker.created_at_utc.as_deref(),
            Some("2026-05-21T11:17:13Z")
        );
    }

    #[test]
    fn update_channel_config_values_accept_stable_and_dev_only() {
        assert_eq!(
            UpdateChannel::from_config_value("stable"),
            Ok(UpdateChannel::Stable)
        );
        assert_eq!(
            UpdateChannel::from_config_value("DEV"),
            Ok(UpdateChannel::Dev)
        );
        assert!(UpdateChannel::from_config_value("nightly").is_err());
    }

    #[test]
    fn protected_install_preserves_discovered_update_and_release_url() {
        let candidate = UpdateCandidate {
            channel: UpdateChannel::Stable,
            version: "9.9.9".to_owned(),
            git_sha: Some("abcdef123456".to_owned()),
            created_at_utc: "2026-07-23T00:00:00Z".to_owned(),
            target: SOROTTE_GUI_TARGET.to_owned(),
            package: "sorotte-gui-9.9.9-windows-x86_64.zip".to_owned(),
            sha256: "a".repeat(64),
            download_url: "https://example.invalid/sorotte.zip".to_owned(),
            details_url: Some("https://example.invalid/releases/9.9.9".to_owned()),
            source: UpdateCandidateSource::ReleaseAsset,
        };
        let result = LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpdateAvailable,
            message: candidate.summary(),
            url: candidate.details_url.clone(),
            candidate: Some(candidate.clone()),
            self_update_supported: true,
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: true,
        };

        let protected =
            apply_self_update_capability(result, SelfUpdateCapability::RequiresElevation);

        assert_eq!(protected.status, LegacyUpdateCheckStatus::UpdateAvailable);
        assert_eq!(protected.url, candidate.details_url);
        assert_eq!(protected.candidate, Some(candidate));
        assert!(!protected.self_update_supported);
        assert!(protected.message.contains("requires elevation"));
    }

    #[test]
    fn update_capability_rejects_protected_and_unwritable_installs_before_download() {
        let root = temp_update_root("capability");
        let current_exe = root.join(SOROTTE_GUI_EXECUTABLE);

        assert_eq!(
            self_update_capability_for_install_with_probe(&current_exe, false, false, |_| true,),
            SelfUpdateCapability::UnsupportedPlatform
        );
        assert_eq!(
            self_update_capability_for_install_with_probe(&current_exe, true, false, |_| true,),
            SelfUpdateCapability::UnpackagedInstall
        );

        fs::write(root.join(SOROTTE_GUI_INSTALL_MARKER), b"{}")
            .expect("packaged-install marker should be written");
        assert_eq!(
            self_update_capability_for_install_with_probe(&current_exe, true, true, |_| true,),
            SelfUpdateCapability::RequiresElevation
        );
        assert_eq!(
            self_update_capability_for_install_with_probe(&current_exe, true, false, |_| false,),
            SelfUpdateCapability::TargetDirectoryNotWritable
        );
        assert_eq!(
            self_update_capability_for_install_with_probe(&current_exe, true, false, |_| true,),
            SelfUpdateCapability::SupportedWritable
        );

        fs::remove_dir_all(root).expect("capability test root should be removed");
    }

    #[test]
    fn update_semver_comparison_detects_newer_versions() {
        let current = parse_version("0.1.0").expect("current version should parse");
        let candidate = parse_version("v0.2.0").expect("candidate version should parse");

        assert!(candidate > current);
    }

    #[test]
    fn update_manifest_validation_rejects_wrong_target() {
        let manifest = UpdateManifest {
            schema: "sorotte-gui-update-manifest-v1".to_owned(),
            app: "sorotte-gui".to_owned(),
            channel: UpdateChannel::Stable,
            version: "0.2.0".to_owned(),
            git_sha: Some("abcdef".to_owned()),
            created_at_utc: "2026-05-20T00:00:00Z".to_owned(),
            target: "linux-x86_64".to_owned(),
            package: "sorotte-gui-0.2.0-windows-x86_64.zip".to_owned(),
            sha256: "a".repeat(64),
        };

        let error = validate_manifest(&manifest, UpdateChannel::Stable)
            .expect_err("wrong target should fail validation");

        assert!(error.contains("target"));
    }

    #[test]
    fn update_manifest_package_must_be_one_normal_basename() {
        assert!(normal_package_basename(
            "sorotte-gui-0.2.0-windows-x86_64.zip"
        ));
        for unsafe_name in [
            ".",
            "..",
            "/sorotte-gui-0.2.0-windows-x86_64.zip",
            r"C:\sorotte-gui-0.2.0-windows-x86_64.zip",
            "sorotte-gui-../payload-windows-x86_64.zip",
            r"sorotte-gui-..\payload-windows-x86_64.zip",
            "sorotte-gui-payload:stream-windows-x86_64.zip",
        ] {
            assert!(
                !normal_package_basename(unsafe_name),
                "unsafe package name should be rejected: {unsafe_name:?}"
            );
        }

        let base = UpdateManifest {
            schema: "sorotte-gui-update-manifest-v1".to_owned(),
            app: "sorotte-gui".to_owned(),
            channel: UpdateChannel::Stable,
            version: "0.2.0".to_owned(),
            git_sha: Some("abcdef".to_owned()),
            created_at_utc: "2026-05-20T00:00:00Z".to_owned(),
            target: SOROTTE_GUI_TARGET.to_owned(),
            package: String::new(),
            sha256: "a".repeat(64),
        };
        for unsafe_name in [
            "sorotte-gui-../payload-windows-x86_64.zip",
            r"sorotte-gui-..\payload-windows-x86_64.zip",
            "sorotte-gui-payload:stream-windows-x86_64.zip",
        ] {
            let manifest = UpdateManifest {
                package: unsafe_name.to_owned(),
                ..base.clone()
            };
            let error = validate_manifest(&manifest, UpdateChannel::Stable)
                .expect_err("manifest traversal package must fail validation");
            assert!(error.contains("unexpected name"));
        }
    }

    #[test]
    fn update_dev_artifact_selection_ignores_expired_artifacts() {
        let artifacts = vec![
            GitHubArtifact {
                name: "sorotte-gui-windows-x86_64".to_owned(),
                archive_download_url: "https://example.invalid/old.zip".to_owned(),
                expired: true,
                created_at: "2026-05-20T10:00:00Z".to_owned(),
                workflow_run: Some(GitHubWorkflowRun {
                    head_sha: Some("old".to_owned()),
                    html_url: None,
                }),
            },
            GitHubArtifact {
                name: "sorotte-gui-windows-x86_64".to_owned(),
                archive_download_url: "https://example.invalid/new.zip".to_owned(),
                expired: false,
                created_at: "2026-05-20T09:00:00Z".to_owned(),
                workflow_run: Some(GitHubWorkflowRun {
                    head_sha: Some("new".to_owned()),
                    html_url: None,
                }),
            },
        ];

        let selected = select_newest_dev_artifact(&artifacts)
            .expect("non-expired matching artifact should be selected");

        assert_eq!(
            selected.archive_download_url,
            "https://example.invalid/new.zip"
        );
    }

    #[test]
    fn update_checksum_verification_rejects_mismatches() {
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        validate_sha256_bytes(b"abc", expected).expect("known SHA-256 should match");
        let error = validate_sha256_bytes(b"abcd", expected)
            .expect_err("mismatched payload should fail checksum verification");

        assert!(error.contains("mismatch"));
    }

    #[test]
    fn update_helper_receives_original_package_and_authenticated_digest() {
        let digest = "a".repeat(64);
        let staged = StagedUpdate {
            candidate: UpdateCandidate {
                channel: UpdateChannel::Stable,
                version: "0.2.4".to_owned(),
                git_sha: None,
                created_at_utc: "2026-07-22T00:00:00Z".to_owned(),
                target: SOROTTE_GUI_TARGET.to_owned(),
                package: "sorotte-gui-0.2.4-windows-x86_64.zip".to_owned(),
                sha256: digest.clone(),
                download_url: "https://example.invalid/update.zip".to_owned(),
                details_url: None,
                source: UpdateCandidateSource::ReleaseAsset,
            },
            package_path: "C:/updates/original.zip".to_owned(),
            source_dir: "C:/updates/mutable-extracted".to_owned(),
            updater_path: "C:/Sorotte/sorotte-gui-updater.exe".to_owned(),
            target_exe_path: "C:/Sorotte/sorotte-gui.exe".to_owned(),
            backup_dir: "C:/updates/mutable-backup".to_owned(),
            log_path: "C:/updates/update.log".to_owned(),
            restart: true,
        };

        let args = staged_update_helper_args(&staged, Path::new("C:/Sorotte"), "123");

        assert!(
            args.windows(2)
                .any(|pair| pair == ["--package", staged.package_path.as_str()])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--package-sha256", digest.as_str()])
        );
        assert!(!args.iter().any(|arg| arg == "--source-dir"));
        assert!(!args.iter().any(|arg| arg == "--backup-dir"));
    }

    #[cfg(windows)]
    #[test]
    fn pending_journal_reentry_uses_recovery_only_updater_protocol() {
        let args = pending_update_recovery_args(
            Path::new("C:/Sorotte"),
            "123",
            Path::new("C:/Temp/recovery.log"),
        );

        assert!(args.iter().any(|arg| arg == "--recover"));
        assert!(args.iter().any(|arg| arg == "--restart"));
        assert!(args.windows(2).any(|pair| pair == ["--pid", "123"]));
        assert!(!args.iter().any(|arg| arg == "--package"));
        assert!(!args.iter().any(|arg| arg == "--source-dir"));
    }

    #[test]
    fn update_staging_cleanup_removes_stale_entries_and_keeps_active_stage() {
        let updates_root = temp_update_root("staging-cleanup");
        let active_stage_dir = updates_root.join("stable-2026-05-22T00-00-00Z");
        let stale_stage_dir = updates_root.join("dev-2026-05-21T00-00-00Z");
        let stale_file = updates_root.join("orphaned-package.zip");
        fs::create_dir_all(active_stage_dir.join("extracted"))
            .expect("active stage should be created");
        fs::create_dir_all(stale_stage_dir.join("backup")).expect("stale stage should be created");
        fs::write(
            stale_stage_dir.join("backup").join("sorotte-gui.exe"),
            b"old",
        )
        .expect("stale stage file should be written");
        fs::write(&stale_file, b"old").expect("stale root file should be written");

        cleanup_updates_root(&updates_root, &active_stage_dir)
            .expect("stale update entries should be removed");

        assert!(active_stage_dir.exists());
        assert!(!stale_stage_dir.exists());
        assert!(!stale_file.exists());
        fs::remove_dir_all(&updates_root).expect("test update root should be removed");
    }

    #[test]
    fn update_staging_root_cleanup_removes_completed_update_folder() {
        let config_root = temp_update_root("staging-root-cleanup");
        let updates_root = config_root.join("updates");
        let completed_stage_dir = updates_root.join("stable-2026-05-22T00-00-00Z");
        fs::create_dir_all(completed_stage_dir.join("backup"))
            .expect("completed update stage should be created");
        fs::write(
            completed_stage_dir.join("sorotte-gui-updater.log"),
            b"update completed",
        )
        .expect("completed update log should be written");

        cleanup_update_staging_root(Some(&config_root))
            .expect("completed update folder should be removed");

        assert!(!updates_root.exists());
        fs::remove_dir_all(&config_root).expect("test config root should be removed");
    }

    #[cfg(windows)]
    #[test]
    fn update_staging_root_cleanup_rejects_updates_junction_without_deleting_target() {
        let root = temp_update_root("staging-root-junction");
        let config_root = root.join("config");
        let outside_root = root.join("outside");
        fs::create_dir_all(&config_root).expect("config root should be created");
        fs::create_dir_all(&outside_root).expect("outside root should be created");
        let canary = outside_root.join("must-not-be-deleted.txt");
        fs::write(&canary, b"outside").expect("outside canary should be written");
        let updates_root = config_root.join("updates");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&updates_root)
            .arg(&outside_root)
            .status()
            .expect("junction command should start");
        assert!(status.success(), "test junction should be created");

        let result = cleanup_update_staging_root(Some(&config_root));

        assert!(
            canary.is_file(),
            "startup cleanup must not traverse the updates junction and delete external contents; got {result:?}"
        );
        assert!(
            result.is_err(),
            "startup cleanup must reject an updates junction; got {result:?}"
        );

        let _ = fs::remove_dir(&updates_root);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_update_stage_cleanup_removes_partial_current_stage() {
        let updates_root = temp_update_root("failed-stage-cleanup");
        let stage_dir = updates_root.join("dev-2026-05-22T00-00-00Z");
        fs::create_dir_all(stage_dir.join("artifact")).expect("partial stage should be created");
        fs::write(stage_dir.join("artifact").join("partial.zip"), b"partial")
            .expect("partial artifact should be written");

        let error = cleanup_failed_stage_dir(&stage_dir, "download failed".to_owned());

        assert_eq!(error, "download failed");
        assert!(!stage_dir.exists());
        fs::remove_dir_all(&updates_root).expect("test update root should be removed");
    }

    #[test]
    fn update_zip_path_rejects_traversal_entries() {
        assert_eq!(
            safe_zip_relative_path("sorotte-gui.exe"),
            Some(std::path::PathBuf::from("sorotte-gui.exe"))
        );
        assert!(safe_zip_relative_path("../sorotte-gui.exe").is_none());
        assert!(safe_zip_relative_path("C:/Windows/sorotte-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"C:\Windows\sorotte-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"\Windows\sorotte-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"bin\..\sorotte-gui.exe").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_update_elevation_detects_program_files_paths_case_insensitively() {
        assert!(path_is_equal_or_child_case_insensitive(
            Path::new(r"C:\Program Files\Sorotte"),
            Path::new(r"c:\program files")
        ));
        assert!(path_is_equal_or_child_case_insensitive(
            Path::new(r"C:\Program Files\Sorotte"),
            Path::new(r"\\?\C:\Program Files")
        ));
        assert!(!path_is_equal_or_child_case_insensitive(
            Path::new(r"C:\Program Files Other\Sorotte"),
            Path::new(r"C:\Program Files")
        ));
    }

    #[test]
    fn wordpress_public_server_response_cleanup_matches_python_rules() {
        let cleaned = sanitize_wordpress_public_server_response(
            "<p>[[' Primary ', ' syncplay.pl:8999 '], ['&#8220;Quoted&#8221;', 'beta.example:9000']]</p>\r\n",
        );
        assert_eq!(
            cleaned,
            "[[' Primary ', ' syncplay.pl:8999 '], [''Quoted'', 'beta.example:9000']]"
        );
    }

    #[test]
    fn public_server_response_parser_accepts_legacy_python_list_format() {
        let parsed = parse_public_server_response(
            "<p>[[' Primary ', ' syncplay.pl:8999 '], ['Backup', 'backup.example:9000']]</p>",
        )
        .expect("legacy public-server list should parse");

        assert_eq!(
            parsed,
            vec![
                (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
                ("Backup".to_owned(), "backup.example:9000".to_owned()),
            ]
        );
    }

    #[test]
    fn public_server_response_parser_rejects_empty_results() {
        let error = parse_public_server_response("[]").expect_err("empty list should fail");
        assert!(error.contains("returned no servers"));
    }

    #[test]
    fn wordpress_update_check_response_cleanup_matches_python_rules() {
        let cleaned = sanitize_wordpress_update_check_response(
            "<p>{&#8220;version-status&#8221;: &#8220;uptodate&#8221;}</p>\r\n",
        );
        assert_eq!(cleaned, "{\"version-status\": \"uptodate\"}");
    }

    #[test]
    fn update_check_response_parser_accepts_legacy_json_and_public_servers() {
        let parsed = parse_update_check_response(
            r#"<p>{"version-status":"updateavailale","version-message":"New build available.","version-url":"https://syncplay.pl/download/","public-servers":"[['Primary','syncplay.pl:8999']]"}</p>"#,
            Some("en"),
            true,
        )
        .expect("legacy update response should parse");

        assert_eq!(parsed.status, LegacyUpdateCheckStatus::UpdateAvailable);
        assert_eq!(parsed.message, "New build available.");
        assert_eq!(parsed.url.as_deref(), Some("https://syncplay.pl/download/"));
        assert_eq!(
            parsed.public_servers,
            Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
        );
    }

    #[test]
    fn update_check_response_parser_falls_back_to_default_failure_message_for_unknown_status() {
        let parsed =
            parse_update_check_response(r#"{"version-status":"mystery"}"#, Some("en"), true)
                .expect("unknown status should still parse");

        assert_eq!(
            parsed.status,
            LegacyUpdateCheckStatus::Unknown("mystery".to_owned())
        );
        assert_eq!(
            parsed.message,
            default_update_check_message(
                &LegacyUpdateCheckStatus::Unknown("mystery".to_owned()),
                Some("en"),
            )
        );
        assert_eq!(parsed.url.as_deref(), Some("https://syncplay.pl/download/"));
    }

    #[test]
    fn public_server_request_uses_selected_language_query_parameter() {
        let (url, request_handle) = spawn_single_request_server("[['Primary','syncplay.pl:8999']]");

        let parsed = fetch_public_servers_from_url(&url, Some("fr"))
            .expect("public-server request should parse the server response");

        assert_eq!(
            parsed,
            vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]
        );
        let request_line = request_handle
            .join()
            .expect("request capture thread should complete");
        assert!(request_line.contains("language=fr"));
    }

    #[test]
    fn update_check_request_uses_selected_language_query_parameter_and_localizes_defaults() {
        let (url, request_handle) = spawn_single_request_server(r#"{"version-status":"uptodate"}"#);

        let parsed = fetch_update_check_result_from_url(&url, Some("fr"), true)
            .expect("update-check request should parse the server response");

        assert_eq!(parsed.status, LegacyUpdateCheckStatus::UpToDate);
        assert_eq!(parsed.message, "Sorotte est a jour");
        let request_line = request_handle
            .join()
            .expect("request capture thread should complete");
        assert!(request_line.contains("language=fr"));
        assert!(request_line.contains("userInitiated=True"));
    }

    #[test]
    fn automatic_update_check_runs_when_timestamp_is_missing_or_stale() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let stale = super::legacy_utc_timestamp_string_legacy_compatible(
            now - Duration::from_secs(super::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS + 1),
        );
        let fresh = super::legacy_utc_timestamp_string_legacy_compatible(
            now - Duration::from_secs(super::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS - 1),
        );

        assert!(should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: None,
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(stale),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(!should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(fresh),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(!should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(false),
                last_checked_for_updates: Some(
                    super::legacy_utc_timestamp_string_legacy_compatible(SystemTime::now())
                ),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
    }

    #[test]
    fn dev_update_order_uses_authoritative_sha_before_non_monotonic_git_timestamp() {
        assert!(!super::dev_candidate_is_newer(
            Some("same-main-tip"),
            "2026-07-28T13:00:00Z",
            Some("same-main-tip"),
            Some("2026-07-28T12:00:00Z"),
        ));
        assert!(!super::dev_candidate_is_newer(
            Some("ABCDEF"),
            "2026-07-28T13:00:00Z",
            Some("abcdef"),
            Some("2026-07-28T12:00:00Z"),
        ));
        assert!(super::dev_candidate_is_newer(
            Some("new-main-tip"),
            "2026-07-28T11:00:00Z",
            Some("old-main-tip"),
            Some("2026-07-28T12:00:00Z"),
        ));
        assert!(!super::dev_candidate_is_newer(
            None,
            "2026-07-28T11:00:00Z",
            Some("old-main-tip"),
            Some("2026-07-28T12:00:00Z"),
        ));
        assert!(super::dev_candidate_is_newer(
            Some("new-main-tip"),
            "2026-07-28T13:00:00Z",
            None,
            Some("2026-07-28T12:00:00Z"),
        ));
    }
}
