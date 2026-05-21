use std::{
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
use syncplay_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;
use syncplay_client_app::app_boundary::persistence::parse_serialized_public_servers_list_legacy_compatible;
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use zip::ZipArchive;

const LEGACY_SYNCPLAY_VERSION: &str = "1.7.5";
const LEGACY_SYNCPLAY_MILESTONE: &str = "Yoitsu";
const LEGACY_SYNCPLAY_RELEASE_NUMBER: &str = "116";
const LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS: u64 = 7 * 86_400;
#[cfg(test)]
const LEGACY_SYNCPLAY_VERSION_STATUS_UP_TO_DATE: &str = "uptodate";
#[cfg(test)]
const LEGACY_SYNCPLAY_VERSION_STATUS_UPDATE_AVAILABLE: &str = "updateavailale";
const SYNCPLAY_PUBLIC_SERVER_LIST_URL: &str = "https://syncplay.pl/listpublicservers";
#[cfg(test)]
const SYNCPLAY_DOWNLOAD_URL: &str = "https://syncplay.pl/download/";
const GITHUB_RELEASES_PAGE_URL: &str =
    "https://github.com/ropbet-radbyt/syncplay-rs-downloads/releases";
const GITHUB_RELEASE_LATEST_URL: &str =
    "https://api.github.com/repos/ropbet-radbyt/syncplay-rs-downloads/releases/latest";
const GITHUB_DEV_RELEASE_URL: &str = "https://api.github.com/repos/ropbet-radbyt/syncplay-rs-downloads/releases/tags/syncplay-gui-dev";
const SYNCPLAY_GUI_APP_NAME: &str = "syncplay-gui";
const SYNCPLAY_UPDATE_MANIFEST_NAME: &str = "syncplay-update-manifest.json";
const SYNCPLAY_GUI_TARGET: &str = "windows-x86_64";
const SYNCPLAY_GUI_RELEASE_PACKAGE_SUFFIX: &str = "-windows-x86_64.zip";
const SYNCPLAY_GUI_DEV_ARTIFACT_NAME: &str = "syncplay-gui-windows-x86_64";
const SYNCPLAY_GUI_INSTALL_MARKER: &str = "syncplay-install.json";
const SYNCPLAY_GUI_EXECUTABLE: &str = "syncplay-gui.exe";
const SYNCPLAY_GUI_UPDATER_EXECUTABLE: &str = "syncplay-gui-updater.exe";
const SYNCPLAY_PUBLIC_SERVER_LIST_URL_ENV: &str = "SYNCPLAY_GUI_PUBLIC_SERVER_LIST_URL";
const SYNCPLAY_PUBLIC_SERVER_LIST_RESPONSE_ENV: &str = "SYNCPLAY_GUI_PUBLIC_SERVER_LIST_RESPONSE";
const SYNCPLAY_UPDATE_CHECK_RESPONSE_ENV: &str = "SYNCPLAY_GUI_UPDATE_CHECK_RESPONSE";
const SYNCPLAY_GITHUB_RELEASE_LATEST_URL_ENV: &str = "SYNCPLAY_GUI_GITHUB_RELEASE_LATEST_URL";
const SYNCPLAY_GITHUB_DEV_RELEASE_URL_ENV: &str = "SYNCPLAY_GUI_GITHUB_DEV_RELEASE_URL";
const SYNCPLAY_GITHUB_ARTIFACTS_URL_ENV: &str = "SYNCPLAY_GUI_GITHUB_ARTIFACTS_URL";
const SYNCPLAY_GUI_UPDATE_CHANNEL_ENV: &str = "SYNCPLAY_GUI_UPDATE_CHANNEL";
const SYNCPLAY_GUI_GITHUB_TOKEN_ENV: &str = "SYNCPLAY_GUI_GITHUB_TOKEN";
const SYNCPLAY_GUI_BUILD_GIT_SHA_ENV: &str = "SYNCPLAY_GUI_BUILD_GIT_SHA";
const SYNCPLAY_GUI_BUILD_CREATED_AT_UTC_ENV: &str = "SYNCPLAY_GUI_BUILD_CREATED_AT_UTC";
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
    fn from_env() -> Result<Self, String> {
        match std::env::var(SYNCPLAY_GUI_UPDATE_CHANNEL_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .as_deref()
        {
            None | Some("stable") => Ok(Self::Stable),
            Some("dev") => Ok(Self::Dev),
            Some(other) => Err(format!(
                "{SYNCPLAY_GUI_UPDATE_CHANNEL_ENV} must be stable or dev, got {other:?}"
            )),
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
            UpdateChannel::Stable => format!("Syncplay GUI {} is available.", self.version),
            UpdateChannel::Dev => {
                let sha = self
                    .git_sha
                    .as_deref()
                    .map(short_git_sha)
                    .unwrap_or("unknown");
                format!("A newer Syncplay GUI dev build is available ({sha}).")
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
    if let Some(body) = env_response_override(SYNCPLAY_PUBLIC_SERVER_LIST_RESPONSE_ENV) {
        return parse_public_server_response(&body).map_err(|error| {
            format!(
                "{}\n-----\n{}",
                error,
                public_server_list_failed_message(language)
            )
        });
    }
    let url = std::env::var(SYNCPLAY_PUBLIC_SERVER_LIST_URL_ENV)
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
) -> LegacyUpdateCheckResult {
    let checked_at_utc =
        legacy_utc_timestamp_string_legacy_compatible(std::time::SystemTime::now());
    match check_for_github_update(language, user_initiated) {
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
            message: "Update downloaded and staged. Restart Syncplay to apply it.".to_owned(),
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
            message: "Update helper started. Syncplay will close and restart after replacement."
                .to_owned(),
        },
        Err(error) => UpdateApplyLaunchResult {
            success: false,
            message: error,
        },
    }
}

fn check_for_github_update(
    language: Option<&str>,
    _user_initiated: bool,
) -> Result<LegacyUpdateCheckResult, String> {
    if !update_supported_platform() {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: "GitHub self-update is currently supported for Windows x64 GUI packages only."
                .to_owned(),
            url: None,
            candidate: None,
            self_update_supported: false,
            public_servers: None,
            checked_at_utc: String::new(),
            user_initiated: false,
        });
    }

    if let Some(body) = env_response_override(SYNCPLAY_UPDATE_CHECK_RESPONSE_ENV)
        && let Some(result) = github_update_response_override_result(&body, language)?
    {
        return Ok(result);
    }

    let channel = UpdateChannel::from_env()?;
    match channel {
        UpdateChannel::Stable => check_stable_release_update(language),
        UpdateChannel::Dev => check_dev_update(language),
    }
}

fn check_stable_release_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    let client = github_http_client()?;
    let release_url = env_trimmed(SYNCPLAY_GITHUB_RELEASE_LATEST_URL_ENV)
        .unwrap_or_else(|| GITHUB_RELEASE_LATEST_URL.to_owned());
    let release: GitHubRelease = github_get_json(&client, &release_url)?;
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
    if env_trimmed(SYNCPLAY_GITHUB_ARTIFACTS_URL_ENV).is_some() {
        check_dev_artifact_update(language)
    } else {
        check_dev_release_update(language)
    }
}

fn check_dev_release_update(language: Option<&str>) -> Result<LegacyUpdateCheckResult, String> {
    let client = github_http_client()?;
    let release_url = env_trimmed(SYNCPLAY_GITHUB_DEV_RELEASE_URL_ENV)
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
    let artifacts_url = env_trimmed(SYNCPLAY_GITHUB_ARTIFACTS_URL_ENV)
        .ok_or_else(|| format!("{SYNCPLAY_GITHUB_ARTIFACTS_URL_ENV} must be set"))?;
    let response: GitHubArtifactsResponse = github_get_json(&client, &artifacts_url)?;
    let Some(artifact) = select_newest_dev_artifact(&response.artifacts) else {
        return Ok(LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::UpToDate,
            message: github_update_up_to_date_message(language, UpdateChannel::Dev),
            url: Some("https://github.com/ropbet-radbyt/syncplay-rs/actions".to_owned()),
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
        target: SYNCPLAY_GUI_TARGET.to_owned(),
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
    if settings.check_for_updates_automatically != Some(true) {
        return false;
    }
    let Some(last_checked) = settings
        .last_checked_for_updates
        .as_deref()
        .and_then(parse_legacy_utc_timestamp_legacy_compatible)
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
    if let Some(token) = env_trimmed(SYNCPLAY_GUI_GITHUB_TOKEN_ENV) {
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
    if let Some(token) = env_trimmed(SYNCPLAY_GUI_GITHUB_TOKEN_ENV) {
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
        .find(|asset| asset.name == SYNCPLAY_UPDATE_MANIFEST_NAME)
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
    if manifest.schema.trim() != "syncplay-gui-update-manifest-v1" {
        return Err(format!(
            "unsupported update manifest schema {:?}",
            manifest.schema
        ));
    }
    if manifest.app.trim() != SYNCPLAY_GUI_APP_NAME {
        return Err(format!("update manifest is for app {:?}", manifest.app));
    }
    if manifest.channel != expected_channel {
        return Err(format!(
            "update manifest channel {} does not match requested {} channel",
            manifest.channel.label(),
            expected_channel.label()
        ));
    }
    if manifest.target.trim() != SYNCPLAY_GUI_TARGET {
        return Err(format!(
            "update manifest target {} does not match {}",
            manifest.target, SYNCPLAY_GUI_TARGET
        ));
    }
    if !manifest.package.starts_with("syncplay-gui-")
        || !manifest
            .package
            .ends_with(SYNCPLAY_GUI_RELEASE_PACKAGE_SUFFIX)
    {
        return Err(format!(
            "update manifest package has unexpected name {}",
            manifest.package
        ));
    }
    validate_sha256_hex(&manifest.sha256)?;
    Ok(())
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
    let expected =
        format!("syncplay-gui-{normalized_version}{SYNCPLAY_GUI_RELEASE_PACKAGE_SUFFIX}");
    assets.iter().find(|asset| asset.name == expected)
}

fn select_newest_dev_artifact(artifacts: &[GitHubArtifact]) -> Option<&GitHubArtifact> {
    artifacts
        .iter()
        .filter(|artifact| artifact.name == SYNCPLAY_GUI_DEV_ARTIFACT_NAME)
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
    let current_sha = env_trimmed(SYNCPLAY_GUI_BUILD_GIT_SHA_ENV);
    if let Some(candidate_sha) = candidate_sha
        && Some(candidate_sha.to_owned()) == current_sha
    {
        return false;
    }
    match env_trimmed(SYNCPLAY_GUI_BUILD_CREATED_AT_UTC_ENV) {
        Some(current_created_at) => candidate_created_at > current_created_at.as_str(),
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

fn self_update_supported_current_install() -> bool {
    if !update_supported_platform() {
        return false;
    }
    let Ok(current_exe) = std::env::current_exe() else {
        return false;
    };
    current_exe
        .parent()
        .map(|parent| parent.join(SYNCPLAY_GUI_INSTALL_MARKER).is_file())
        .unwrap_or(false)
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
            "https://example.invalid/syncplay-gui-update.zip".to_owned(),
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
        "Syncplay is up to date",
        "Syncplay ist auf dem neuesten Stand",
        "Syncplay esta actualizado",
        "Syncplay estas gxisdata",
        "Syncplay on ajan tasalla",
        "Syncplay est a jour",
        "Syncplay e aggiornato",
        "O Syncplay esta atualizado",
        "Syncplay guncel",
        "Syncplay obnovlen do poslednei versii",
        "Syncplay yi shi zuixin banben",
        "Syncplayneun choesin sangtaeimnida",
    );
    match channel {
        UpdateChannel::Stable => base.to_owned(),
        UpdateChannel::Dev => format!("{base} on the dev channel"),
    }
}

fn github_update_check_failed_message(language: Option<&str>) -> String {
    localized_literal(
        language,
        "Could not check GitHub for Syncplay GUI updates. Open the GitHub releases page to check manually.",
        "GitHub konnte nicht auf Syncplay-GUI-Updates geprueft werden. Oeffnen Sie die GitHub-Releases-Seite zur manuellen Pruefung.",
        "No se pudo comprobar GitHub para actualizaciones de la GUI de Syncplay. Abra la pagina de lanzamientos de GitHub para comprobar manualmente.",
        "Ne eblis kontroli GitHub por Syncplay GUI gxisdatigoj. Malfermu la GitHub eldonpaghon por kontroli mane.",
        "GitHubista ei voitu tarkistaa Syncplay GUI -paivityksia. Tarkista paivitykset kasin GitHub-julkaisusivulta.",
        "Impossible de verifier les mises a jour de l'interface Syncplay sur GitHub. Ouvrez la page des versions GitHub pour verifier manuellement.",
        "Impossibile controllare GitHub per aggiornamenti della GUI di Syncplay. Apri la pagina release di GitHub per controllare manualmente.",
        "Nao foi possivel verificar atualizacoes da GUI do Syncplay no GitHub. Abra a pagina de lancamentos do GitHub para verificar manualmente.",
        "GitHub'da Syncplay GUI guncellemeleri denetlenemedi. Elle denetlemek icin GitHub surumleri sayfasini acin.",
        "Ne udalos proverit GitHub na obnovleniia Syncplay GUI. Otkroite stranicu vypuskov GitHub dlia ruchnoi proverki.",
        "Wu fa zai GitHub shang jiancha Syncplay GUI gengxin. Qing dakai GitHub fabu yemian shoudong jiancha.",
        "GitHub-eseo Syncplay GUI eobdeiteureul hwaginhal su eopseotseumnida. susdong hwagineul wihae GitHub baepo peijireul yeoreojuseyo.",
    )
    .to_owned()
}

fn download_and_stage_update_inner(
    candidate: &UpdateCandidate,
    gui_config_root: Option<&Path>,
) -> Result<StagedUpdate, String> {
    if !update_supported_platform() {
        return Err("Self-update is only supported for Windows x64 GUI packages.".to_owned());
    }
    if !self_update_supported_current_install() {
        return Err(
            "This Syncplay GUI build is not a packaged install; update checks are allowed, but self-replacement is disabled."
                .to_owned(),
        );
    }
    let Some(gui_config_root) = gui_config_root else {
        return Err(
            "Cannot stage update because the Syncplay GUI config root is unavailable.".to_owned(),
        );
    };
    let client = github_http_client()?;
    let updates_root = gui_config_root.join("Syncplay").join("updates");
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

    let downloaded_bytes = github_download_bytes(&client, &candidate.download_url)?;
    let (package_bytes, staged_candidate) = match candidate.source {
        UpdateCandidateSource::ReleaseAsset => {
            validate_sha256_bytes(&downloaded_bytes, &candidate.sha256)?;
            (downloaded_bytes, candidate.clone())
        }
        UpdateCandidateSource::ActionsArtifact => {
            let artifact_dir = stage_dir.join("artifact");
            extract_zip_bytes_safe(&downloaded_bytes, &artifact_dir)?;
            let manifest_path = artifact_dir.join(SYNCPLAY_UPDATE_MANIFEST_NAME);
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
    let updater_path = source_dir.join(SYNCPLAY_GUI_UPDATER_EXECUTABLE);
    let log_path = stage_dir.join("syncplay-gui-updater.log");
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
    if candidate.target != SYNCPLAY_GUI_TARGET {
        return Err(format!(
            "update package targets {}, but this build expects {}",
            candidate.target, SYNCPLAY_GUI_TARGET
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
    let actual = format!("{:x}", hasher.finalize());
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
    for required in [SYNCPLAY_GUI_EXECUTABLE, SYNCPLAY_GUI_UPDATER_EXECUTABLE] {
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
    if !update_supported_platform() {
        return Err("Self-update is only supported for Windows x64 GUI packages.".to_owned());
    }
    if !self_update_supported_current_install() {
        return Err(
            "This Syncplay GUI build is not a packaged install; self-replacement is disabled."
                .to_owned(),
        );
    }
    let current_pid = std::process::id().to_string();
    let target_exe = PathBuf::from(&staged_update.target_exe_path);
    let target_dir = target_exe
        .parent()
        .ok_or_else(|| "current GUI executable has no parent directory".to_owned())?;
    let mut command = Command::new(&staged_update.updater_path);
    command
        .arg("--pid")
        .arg(current_pid)
        .arg("--source-dir")
        .arg(&staged_update.source_dir)
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--target-exe")
        .arg(SYNCPLAY_GUI_EXECUTABLE)
        .arg("--backup-dir")
        .arg(&staged_update.backup_dir)
        .arg("--log")
        .arg(&staged_update.log_path);
    if staged_update.restart {
        command.arg("--restart");
    }
    command
        .spawn()
        .map_err(|error| format!("failed to launch update helper: {error}"))?;
    Ok(())
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
        .user_agent(format!("syncplay-rs-gui/{}", env!("CARGO_PKG_VERSION")))
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
            "Syncplay is up to date",
            "Syncplay ist auf dem neuesten Stand",
            "Syncplay esta actualizado",
            "Syncplay estas gxisdata",
            "Syncplay on ajan tasalla",
            "Syncplay est a jour",
            "Syncplay e aggiornato",
            "O Syncplay esta atualizado",
            "Syncplay guncel",
            "Syncplay obnovlen do poslednei versii",
            "Syncplay yi shi zuixin banben",
            "Syncplayneun choesin sangtaeimnida",
        )
        .to_owned(),
        LegacyUpdateCheckStatus::UpdateAvailable => localized_literal(
            language,
            "A new version of Syncplay is available. Do you want to visit the release page?",
            "Eine neue Version von Syncplay ist verfuegbar. Moechten Sie die Release-Seite besuchen?",
            "Hay una nueva version de Syncplay disponible. Desea visitar la pagina de lanzamiento?",
            "Nova versio de Syncplay disponeblas. Chu vi volas viziti la eldonan paghon?",
            "Uusi Syncplay-versio on saatavilla. Haluatko avata julkaisusivun?",
            "Une nouvelle version de Syncplay est disponible. Voulez-vous visiter la page de publication?",
            "E disponibile una nuova versione di Syncplay. Vuoi visitare la pagina di rilascio?",
            "Uma nova versao do Syncplay esta disponivel. Deseja visitar a pagina de lancamento?",
            "Syncplay'in yeni bir surumu mevcut. Surum sayfasini ziyaret etmek ister misiniz?",
            "Dostupna novaia versiia Syncplay. Otkryt stranicu vypuska?",
            "You xin de Syncplay banben ke yong. Yao fangwen fabu yemian ma?",
            "Syncplay-ui saeroun beojeoni isseumnida. baepo peijireul bangmunhasigesseumnikka?",
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
        "Could not automatically check whether Syncplay {} is up to date. Want to visit https://syncplay.pl/ to manually check for updates?",
        "Es konnte nicht automatisch geprueft werden, ob Syncplay {} aktuell ist. Moechten Sie https://syncplay.pl/ besuchen, um manuell nach Updates zu suchen?",
        "No se pudo comprobar automaticamente si Syncplay {} esta actualizado. Desea visitar https://syncplay.pl/ para comprobar manualmente si hay actualizaciones?",
        "Ne eblis auxtomate kontroli chu Syncplay {} estas gxisdata. Chu vi volas viziti https://syncplay.pl/ por mane kontroli gxisdatigojn?",
        "Ei voitu tarkistaa automaattisesti, onko Syncplay {} ajan tasalla. Haluatko kayda osoitteessa https://syncplay.pl/ tarkistaaksesi paivitykset manuaalisesti?",
        "Impossible de verifier automatiquement si Syncplay {} est a jour. Voulez-vous visiter https://syncplay.pl/ pour verifier manuellement les mises a jour?",
        "Impossibile verificare automaticamente se Syncplay {} e aggiornato. Vuoi visitare https://syncplay.pl/ per controllare manualmente gli aggiornamenti?",
        "Nao foi possivel verificar automaticamente se o Syncplay {} esta atualizado. Deseja visitar https://syncplay.pl/ para verificar atualizacoes manualmente?",
        "Syncplay {}'in guncel olup olmadigi otomatik olarak denetlenemedi. Guncellemeleri elle kontrol etmek icin https://syncplay.pl/ adresini ziyaret etmek ister misiniz?",
        "Ne udalos avtomaticheski proverit, obnovlen li Syncplay {}. Hotite pereiti na https://syncplay.pl/ dlia ruchnoi proverki obnovlenii?",
        "Wu fa zidong jiancha Syncplay {} shifou wei zuixin banben. Yao fangwen https://syncplay.pl/ shoudong jiancha gengxin ma?",
        "Syncplay {}ga choesin beojeoninji jadongeuro hwaginhal su eopseotseumnida. susdong-euro eobdeiteureul hwaginhagi wihae https://syncplay.pl/ reul bangmunhasigesseumnikka?",
    )
    .replace("{}", LEGACY_SYNCPLAY_VERSION)
}

#[cfg(test)]
fn localize_wire_update_message(message: &str, language: Option<&str>) -> String {
    let trimmed = message.trim();
    match trimmed {
        "Syncplay is up to date" | "Syncplay is up to date." => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpToDate, language)
        }
        "A new version of Syncplay is available. Do you want to visit the release page?" => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpdateAvailable, language)
        }
        _ => trimmed.to_owned(),
    }
}

fn legacy_utc_timestamp_string_legacy_compatible(now: std::time::SystemTime) -> String {
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
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        GitHubArtifact, GitHubReleaseAsset, GitHubWorkflowRun, LegacyUpdateCheckStatus,
        StoredClientSettingsMvp, UpdateChannel, UpdateManifest, default_update_check_message,
        fetch_public_servers_from_url, fetch_update_check_result_from_url,
        parse_public_server_response, parse_update_check_response, parse_version,
        safe_zip_relative_path, sanitize_wordpress_public_server_response,
        sanitize_wordpress_update_check_response, select_newest_dev_artifact,
        select_stable_gui_release_asset, should_run_automatic_update_check, validate_manifest,
        validate_sha256_bytes,
    };

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

    #[test]
    fn update_stable_asset_selection_uses_versioned_windows_package() {
        let assets = vec![
            GitHubReleaseAsset {
                name: "syncplay-gui-0.2.0-linux-x86_64.zip".to_owned(),
                browser_download_url: "https://example.invalid/linux.zip".to_owned(),
            },
            GitHubReleaseAsset {
                name: "syncplay-gui-0.2.0-windows-x86_64.zip".to_owned(),
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
    fn update_default_release_urls_use_public_downloads_repo() {
        assert!(
            super::GITHUB_RELEASE_LATEST_URL
                .contains("/repos/ropbet-radbyt/syncplay-rs-downloads/releases/latest")
        );
        assert!(
            super::GITHUB_DEV_RELEASE_URL.contains(
                "/repos/ropbet-radbyt/syncplay-rs-downloads/releases/tags/syncplay-gui-dev"
            )
        );
        assert_eq!(
            super::GITHUB_RELEASES_PAGE_URL,
            "https://github.com/ropbet-radbyt/syncplay-rs-downloads/releases"
        );
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
            schema: "syncplay-gui-update-manifest-v1".to_owned(),
            app: "syncplay-gui".to_owned(),
            channel: UpdateChannel::Stable,
            version: "0.2.0".to_owned(),
            git_sha: Some("abcdef".to_owned()),
            created_at_utc: "2026-05-20T00:00:00Z".to_owned(),
            target: "linux-x86_64".to_owned(),
            package: "syncplay-gui-0.2.0-windows-x86_64.zip".to_owned(),
            sha256: "a".repeat(64),
        };

        let error = validate_manifest(&manifest, UpdateChannel::Stable)
            .expect_err("wrong target should fail validation");

        assert!(error.contains("target"));
    }

    #[test]
    fn update_dev_artifact_selection_ignores_expired_artifacts() {
        let artifacts = vec![
            GitHubArtifact {
                name: "syncplay-gui-windows-x86_64".to_owned(),
                archive_download_url: "https://example.invalid/old.zip".to_owned(),
                expired: true,
                created_at: "2026-05-20T10:00:00Z".to_owned(),
                workflow_run: Some(GitHubWorkflowRun {
                    head_sha: Some("old".to_owned()),
                    html_url: None,
                }),
            },
            GitHubArtifact {
                name: "syncplay-gui-windows-x86_64".to_owned(),
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
    fn update_zip_path_rejects_traversal_entries() {
        assert_eq!(
            safe_zip_relative_path("syncplay-gui.exe"),
            Some(std::path::PathBuf::from("syncplay-gui.exe"))
        );
        assert!(safe_zip_relative_path("../syncplay-gui.exe").is_none());
        assert!(safe_zip_relative_path("C:/Windows/syncplay-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"C:\Windows\syncplay-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"\Windows\syncplay-gui.exe").is_none());
        assert!(safe_zip_relative_path(r"bin\..\syncplay-gui.exe").is_none());
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
        assert_eq!(parsed.message, "Syncplay est a jour");
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
}
