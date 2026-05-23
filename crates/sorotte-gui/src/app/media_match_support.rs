use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::{io::Read, time::Duration};

use serde::{Deserialize, Serialize};
use sorotte_media_match::{
    MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFingerprintRecord,
    MediaMatchCacheV1, MediaMatchSettings, MediaMatchTier, MediaMatchToolPaths,
    fingerprint_media_file, normalize_media_path, rank_media_match_candidates,
};

use super::shell_state::{
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchToolHealth,
    media_match_settings_from_stored_settings,
};

#[cfg(windows)]
use zip::ZipArchive;

const MEDIA_MATCH_METADATA_VERSION: u32 = 1;
const MEDIA_MATCH_CACHE_FILE: &str = "media-match-cache-v1.json";
#[cfg(windows)]
const MEDIA_MATCH_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
#[cfg(windows)]
const MEDIA_MATCH_DOWNLOAD_PROGRESS_STEP_BYTES: u64 = 1024 * 1024;
#[cfg(windows)]
const MEDIA_MATCH_DOWNLOAD_BUFFER_BYTES: usize = 64 * 1024;
#[cfg(windows)]
const MEDIA_MATCH_DOWNLOAD_PREALLOC_MAX_BYTES: usize = 128 * 1024 * 1024;
#[cfg(windows)]
const MEDIA_MATCH_USER_AGENT: &str = concat!("sorotte-gui/", env!("CARGO_PKG_VERSION"));
#[cfg(windows)]
const FFMPEG_WINDOWS_ZIP_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
#[cfg(windows)]
const FPCALC_WINDOWS_ZIP_URL: &str = "https://github.com/acoustid/chromaprint/releases/download/v1.5.1/chromaprint-fpcalc-1.5.1-windows-x86_64.zip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaMatchTool {
    Ffmpeg,
    Ffprobe,
    Fpcalc,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MediaMatchToolProgress {
    pub(super) label: String,
    pub(super) detail: Option<String>,
    pub(super) progress_fraction: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MediaMatchIndexRebuildResult {
    pub(super) message: String,
    pub(super) cache_status: String,
    pub(super) current_decision: Option<String>,
    pub(super) last_evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct ManagedMediaMatchMetadata {
    version: u32,
    installed_at_unix_seconds: Option<u64>,
    ffmpeg_version: Option<String>,
    ffprobe_version: Option<String>,
    fpcalc_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaMatchToolProbe {
    path: Option<PathBuf>,
    error: Option<String>,
    status: String,
}

impl MediaMatchTool {
    fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Fpcalc => "fpcalc",
        }
    }

    fn managed_file_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => {
                if cfg!(windows) {
                    "ffmpeg.exe"
                } else {
                    "ffmpeg"
                }
            }
            Self::Ffprobe => {
                if cfg!(windows) {
                    "ffprobe.exe"
                } else {
                    "ffprobe"
                }
            }
            Self::Fpcalc => {
                if cfg!(windows) {
                    "fpcalc.exe"
                } else {
                    "fpcalc"
                }
            }
        }
    }

    fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg | Self::Ffprobe => &["-version"],
            Self::Fpcalc => &["-version"],
        }
    }

    fn assign_version(self, metadata: &mut ManagedMediaMatchMetadata, version: String) {
        match self {
            Self::Ffmpeg => metadata.ffmpeg_version = Some(version),
            Self::Ffprobe => metadata.ffprobe_version = Some(version),
            Self::Fpcalc => metadata.fpcalc_version = Some(version),
        }
    }
}

impl MediaMatchToolProgress {
    fn new(label: impl Into<String>, detail: Option<String>, progress_fraction: f32) -> Self {
        Self {
            label: label.into(),
            detail,
            progress_fraction,
        }
    }
}

pub(super) fn managed_media_match_bin_dir(root: &Path) -> PathBuf {
    root.join("tools").join("media-match").join("bin")
}

fn managed_media_match_metadata_path(root: &Path) -> PathBuf {
    managed_media_match_bin_dir(root).join("metadata.json")
}

fn managed_media_match_cache_path(root: &Path) -> PathBuf {
    root.join(MEDIA_MATCH_CACHE_FILE)
}

pub(super) fn clear_persisted_media_match_cache_at_root(root: &Path) -> Result<(), String> {
    let cache_path = managed_media_match_cache_path(root);
    if cache_path.exists() {
        fs::remove_file(&cache_path).map_err(|error| {
            format!(
                "failed removing media-match cache '{}': {error}",
                cache_path.display()
            )
        })?;
    }
    let metadata_path = managed_media_match_metadata_path(root);
    if metadata_path.exists() {
        fs::remove_file(&metadata_path).map_err(|error| {
            format!(
                "failed removing media-match tool metadata '{}': {error}",
                metadata_path.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn managed_media_match_tool_path(root: &Path, tool: MediaMatchTool) -> PathBuf {
    managed_media_match_bin_dir(root).join(tool.managed_file_name())
}

pub(super) fn probe_media_match_runtime_snapshot(
    root: Option<&Path>,
    settings: &MediaMatchSettings,
) -> GuiMediaMatchRuntimeSnapshot {
    let ffmpeg = probe_tool(root, MediaMatchTool::Ffmpeg);
    let ffprobe = probe_tool(root, MediaMatchTool::Ffprobe);
    let fpcalc = probe_tool(root, MediaMatchTool::Fpcalc);
    let health = media_match_health(&ffmpeg, &ffprobe, &fpcalc);
    let message = media_match_health_message(health, &ffmpeg, &ffprobe, &fpcalc);
    GuiMediaMatchRuntimeSnapshot {
        settings: settings.clone(),
        health,
        message,
        install_supported: cfg!(windows),
        integration_supported: true,
        install_location: root.map(|root| managed_media_match_bin_dir(root).display().to_string()),
        ffmpeg_status: Some(ffmpeg.status),
        ffprobe_status: Some(ffprobe.status),
        fpcalc_status: Some(fpcalc.status),
        cache_status: root.map(media_match_cache_status),
        current_decision: None,
        last_evidence: None,
        open_install_location_available: root.is_some(),
    }
}

pub(super) fn probe_media_match_startup_snapshot(
    root: Option<&Path>,
    settings: Option<&sorotte_client_app::app_boundary::state::StoredClientSettingsMvp>,
) -> GuiMediaMatchRuntimeSnapshot {
    let settings = settings
        .map(media_match_settings_from_stored_settings)
        .unwrap_or_default();
    probe_media_match_runtime_snapshot(root, &settings)
}

pub(super) fn import_managed_media_match_tool_with_progress<F>(
    root: &Path,
    tool: MediaMatchTool,
    source_path: &Path,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    progress(MediaMatchToolProgress::new(
        format!("Importing {}", tool.display_name()),
        Some(source_path.display().to_string()),
        0.15,
    ));
    if !source_path.is_file() {
        return Err(format!(
            "{} import source does not exist: {}",
            tool.display_name(),
            source_path.display()
        ));
    }
    let bin_dir = managed_media_match_bin_dir(root);
    fs::create_dir_all(&bin_dir).map_err(|error| {
        format!(
            "failed creating media-match tool directory '{}': {error}",
            bin_dir.display()
        )
    })?;
    let target = managed_media_match_tool_path(root, tool);
    fs::copy(source_path, &target).map_err(|error| {
        format!(
            "failed importing {} to '{}': {error}",
            tool.display_name(),
            target.display()
        )
    })?;
    progress(MediaMatchToolProgress::new(
        format!("Verifying {}", tool.display_name()),
        Some(target.display().to_string()),
        0.72,
    ));
    let version = probe_executable_version(&target, tool.version_args())?;
    let mut metadata = load_managed_media_match_metadata(root).unwrap_or_default();
    metadata.version = MEDIA_MATCH_METADATA_VERSION;
    metadata.installed_at_unix_seconds = Some(current_unix_seconds());
    tool.assign_version(&mut metadata, version.clone());
    save_managed_media_match_metadata(root, &metadata)?;
    progress(MediaMatchToolProgress::new(
        format!("Imported {}", tool.display_name()),
        Some(version.clone()),
        1.0,
    ));
    Ok(format!(
        "Imported {} for Media Matching: {version}",
        tool.display_name()
    ))
}

pub(super) fn install_or_update_managed_media_match_tools_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    #[cfg(not(windows))]
    {
        let _ = root;
        progress(MediaMatchToolProgress::new(
            "Media Matching tool install unavailable",
            Some("Import ffmpeg, ffprobe, and fpcalc manually on this platform.".to_owned()),
            1.0,
        ));
        Err("Automatic Media Matching tool installation is currently Windows-only.".to_owned())
    }
    #[cfg(windows)]
    {
        let bin_dir = managed_media_match_bin_dir(root);
        fs::create_dir_all(&bin_dir).map_err(|error| {
            format!(
                "failed creating media-match tool directory '{}': {error}",
                bin_dir.display()
            )
        })?;
        let client = media_match_http_client()?;
        let ffmpeg_zip = download_bytes_with_progress(
            &client,
            FFMPEG_WINDOWS_ZIP_URL,
            "Downloading ffmpeg tools",
            0.08,
            0.54,
            &mut progress,
        )?;
        progress(MediaMatchToolProgress::new(
            "Extracting ffmpeg tools",
            Some("Installing ffmpeg.exe and ffprobe.exe.".to_owned()),
            0.55,
        ));
        extract_zip_entry(
            &ffmpeg_zip,
            "ffmpeg.exe",
            &managed_media_match_tool_path(root, MediaMatchTool::Ffmpeg),
        )?;
        extract_zip_entry(
            &ffmpeg_zip,
            "ffprobe.exe",
            &managed_media_match_tool_path(root, MediaMatchTool::Ffprobe),
        )?;
        let fpcalc_zip = download_bytes_with_progress(
            &client,
            FPCALC_WINDOWS_ZIP_URL,
            "Downloading fpcalc",
            0.58,
            0.72,
            &mut progress,
        )?;
        progress(MediaMatchToolProgress::new(
            "Extracting fpcalc",
            Some("Installing fpcalc.exe.".to_owned()),
            0.73,
        ));
        extract_zip_entry(
            &fpcalc_zip,
            "fpcalc.exe",
            &managed_media_match_tool_path(root, MediaMatchTool::Fpcalc),
        )?;

        let mut metadata = ManagedMediaMatchMetadata {
            version: MEDIA_MATCH_METADATA_VERSION,
            installed_at_unix_seconds: Some(current_unix_seconds()),
            ..ManagedMediaMatchMetadata::default()
        };
        for tool in [
            MediaMatchTool::Ffmpeg,
            MediaMatchTool::Ffprobe,
            MediaMatchTool::Fpcalc,
        ] {
            progress(MediaMatchToolProgress::new(
                format!("Verifying {}", tool.display_name()),
                None,
                0.76,
            ));
            let version = probe_executable_version(
                &managed_media_match_tool_path(root, tool),
                tool.version_args(),
            )?;
            tool.assign_version(&mut metadata, version);
        }
        save_managed_media_match_metadata(root, &metadata)?;
        progress(MediaMatchToolProgress::new(
            "Media Matching tools installed",
            Some(bin_dir.display().to_string()),
            1.0,
        ));
        Ok("Installed ffmpeg, ffprobe, and fpcalc for Media Matching.".to_owned())
    }
}

pub(super) fn rebuild_persisted_media_match_index_with_progress<F>(
    root: &Path,
    search_roots: &[PathBuf],
    current_player_path: Option<&str>,
    settings: &MediaMatchSettings,
    mut progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    if !settings.fingerprinting_enabled {
        return Err("Enable Media Matching fingerprinting before rebuilding the index.".to_owned());
    }
    let tools = media_match_tool_paths(root)?;
    let extraction_settings = MediaExtractionSettings::default();
    progress(MediaMatchToolProgress::new(
        "Scanning media-search roots",
        Some(format!("{} roots", search_roots.len())),
        0.05,
    ));
    let candidates = collect_media_match_candidates(search_roots);
    let existing_cache = load_media_match_cache(root).unwrap_or_default();
    let mut next_cache = MediaMatchCacheV1::default();
    let mut reused = 0usize;
    let mut fingerprinted = 0usize;
    let mut skipped = 0usize;
    let total = candidates.len().max(1);

    for (index, path) in candidates.iter().enumerate() {
        let progress_fraction = 0.1 + (0.82 * (index as f32 / total as f32));
        progress(MediaMatchToolProgress::new(
            "Fingerprinting media",
            Some(path.display().to_string()),
            progress_fraction,
        ));
        match cached_or_fresh_media_fingerprint(&existing_cache, path, &tools, &extraction_settings)
        {
            Ok((record, was_reused)) => {
                if was_reused {
                    reused += 1;
                } else {
                    fingerprinted += 1;
                }
                next_cache.insert(record);
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }

    save_media_match_cache(root, &next_cache)?;
    let (current_decision, last_evidence) =
        summarize_current_media_match(current_player_path, &next_cache, settings);
    let cache_status = format!("{} fingerprint records", next_cache.records.len());
    let message = format!(
        "Media Matching indexed {} files ({} reused, {} fingerprinted, {} skipped).",
        candidates.len(),
        reused,
        fingerprinted,
        skipped
    );
    progress(MediaMatchToolProgress::new(
        "Media Matching index rebuilt",
        Some(cache_status.clone()),
        1.0,
    ));

    Ok(MediaMatchIndexRebuildResult {
        message,
        cache_status,
        current_decision,
        last_evidence,
    })
}

fn probe_tool(root: Option<&Path>, tool: MediaMatchTool) -> MediaMatchToolProbe {
    let path = root
        .map(|root| managed_media_match_tool_path(root, tool))
        .filter(|path| path.is_file())
        .or_else(|| find_executable_on_path(tool.managed_file_name()))
        .or_else(|| find_executable_on_path(tool.display_name()));
    let Some(path) = path else {
        return MediaMatchToolProbe {
            path: None,
            error: None,
            status: format!("Missing {}", tool.display_name()),
        };
    };
    match probe_executable_version(&path, tool.version_args()) {
        Ok(version) => MediaMatchToolProbe {
            path: Some(path.clone()),
            error: None,
            status: format!("{} ({})", version, path.display()),
        },
        Err(error) => MediaMatchToolProbe {
            path: Some(path.clone()),
            error: Some(error.clone()),
            status: format!(
                "{} unusable at '{}': {error}",
                tool.display_name(),
                path.display()
            ),
        },
    }
}

fn media_match_health(
    ffmpeg: &MediaMatchToolProbe,
    ffprobe: &MediaMatchToolProbe,
    fpcalc: &MediaMatchToolProbe,
) -> GuiMediaMatchToolHealth {
    if ffmpeg.error.is_some() || ffprobe.error.is_some() || fpcalc.error.is_some() {
        return GuiMediaMatchToolHealth::Broken;
    }
    if ffmpeg.path.is_none() {
        return GuiMediaMatchToolHealth::MissingFfmpeg;
    }
    if ffprobe.path.is_none() {
        return GuiMediaMatchToolHealth::MissingFfprobe;
    }
    if fpcalc.path.is_none() {
        return GuiMediaMatchToolHealth::MissingFpcalc;
    }
    GuiMediaMatchToolHealth::Healthy
}

fn media_match_health_message(
    health: GuiMediaMatchToolHealth,
    ffmpeg: &MediaMatchToolProbe,
    ffprobe: &MediaMatchToolProbe,
    fpcalc: &MediaMatchToolProbe,
) -> Option<String> {
    match health {
        GuiMediaMatchToolHealth::Healthy => None,
        GuiMediaMatchToolHealth::MissingFfmpeg => {
            Some("Media Matching needs ffmpeg for frame extraction.".to_owned())
        }
        GuiMediaMatchToolHealth::MissingFfprobe => {
            Some("Media Matching needs ffprobe for media metadata.".to_owned())
        }
        GuiMediaMatchToolHealth::MissingFpcalc => {
            Some("Media Matching needs fpcalc for Chromaprint audio fingerprints.".to_owned())
        }
        GuiMediaMatchToolHealth::Broken => Some(format!(
            "One or more Media Matching tools could not run: {}; {}; {}",
            ffmpeg.status, ffprobe.status, fpcalc.status
        )),
    }
}

fn media_match_cache_status(root: &Path) -> String {
    let path = managed_media_match_cache_path(root);
    let Ok(contents) = fs::read_to_string(&path) else {
        return "empty".to_owned();
    };
    match serde_json::from_str::<MediaMatchCacheV1>(&contents) {
        Ok(cache) => format!("{} fingerprint records", cache.records.len()),
        Err(error) => format!("unreadable cache: {error}"),
    }
}

fn media_match_tool_paths(root: &Path) -> Result<MediaMatchToolPaths, String> {
    let ffmpeg = probe_tool(Some(root), MediaMatchTool::Ffmpeg);
    let ffprobe = probe_tool(Some(root), MediaMatchTool::Ffprobe);
    let fpcalc = probe_tool(Some(root), MediaMatchTool::Fpcalc);
    let health = media_match_health(&ffmpeg, &ffprobe, &fpcalc);
    if health != GuiMediaMatchToolHealth::Healthy {
        return Err(
            media_match_health_message(health, &ffmpeg, &ffprobe, &fpcalc).unwrap_or_else(|| {
                "Media Matching tools are not ready for fingerprint extraction.".to_owned()
            }),
        );
    }
    Ok(MediaMatchToolPaths {
        ffmpeg: ffmpeg
            .path
            .ok_or_else(|| "Media Matching could not resolve ffmpeg.".to_owned())?,
        ffprobe: ffprobe
            .path
            .ok_or_else(|| "Media Matching could not resolve ffprobe.".to_owned())?,
        fpcalc: fpcalc
            .path
            .ok_or_else(|| "Media Matching could not resolve fpcalc.".to_owned())?,
    })
}

fn collect_media_match_candidates(search_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = search_roots
        .iter()
        .filter(|root| root.is_dir())
        .cloned()
        .collect::<Vec<_>>();
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() && media_match_candidate_extension(&path) {
                files.push(path);
            }
        }
    }
    files.sort_by(|left, right| {
        normalize_media_path(left)
            .cmp(&normalize_media_path(right))
            .then_with(|| left.cmp(right))
    });
    files
}

fn media_match_candidate_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv"
                    | "mp4"
                    | "m4v"
                    | "avi"
                    | "mov"
                    | "webm"
                    | "ogv"
                    | "ts"
                    | "m2ts"
                    | "mpg"
                    | "mpeg"
                    | "wmv"
            )
        })
        .unwrap_or(false)
}

fn cached_or_fresh_media_fingerprint(
    existing_cache: &MediaMatchCacheV1,
    path: &Path,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
) -> Result<(MediaFingerprintRecord, bool), String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "failed reading media metadata '{}': {error}",
            path.display()
        )
    })?;
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    if let Some(record) = existing_cache.get_valid(
        path,
        modified_unix_millis,
        metadata.len(),
        MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings,
    ) {
        return Ok((record.clone(), true));
    }
    fingerprint_media_file(path, tools, extraction_settings)
        .map(|record| (record, false))
        .map_err(|error| error.to_string())
}

fn summarize_current_media_match(
    current_player_path: Option<&str>,
    cache: &MediaMatchCacheV1,
    settings: &MediaMatchSettings,
) -> (Option<String>, Option<String>) {
    let Some(current_player_path) = current_player_path else {
        return (Some("unknown: no current player file".to_owned()), None);
    };
    let normalized_current_path = normalize_media_path(current_player_path);
    let Some(query) = cache.records.get(&normalized_current_path) else {
        return (
            Some("unknown: current player file is not indexed".to_owned()),
            None,
        );
    };
    let ranked = rank_media_match_candidates(
        query,
        cache
            .records
            .values()
            .filter(|record| record.identity.normalized_path != normalized_current_path),
        settings,
    );
    let Some(best) = ranked
        .into_iter()
        .find(|candidate| candidate.decision.tier != MediaMatchTier::Reject)
    else {
        return (
            Some("unknown: no comparable indexed candidates".to_owned()),
            None,
        );
    };
    let tier = media_match_tier_label(best.decision.tier);
    (
        Some(format!("{tier}: {}", best.decision.explanation)),
        Some(format!("best candidate: {}", best.candidate_path)),
    )
}

fn media_match_tier_label(tier: MediaMatchTier) -> &'static str {
    match tier {
        MediaMatchTier::Exact => "exact",
        MediaMatchTier::Strong => "strong",
        MediaMatchTier::Probable => "probable",
        MediaMatchTier::Weak => "weak",
        MediaMatchTier::Reject => "reject",
        MediaMatchTier::Unknown => "unknown",
    }
}

fn find_executable_on_path(file_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(file_name))
        .find(|path| path.is_file())
}

fn probe_executable_version(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(path).args(args).output().map_err(|error| {
        format!(
            "failed to run '{} {}': {error}",
            path.display(),
            args.join(" ")
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "exited with status {}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("version output empty")
        .trim();
    Ok(first_line.to_owned())
}

fn load_managed_media_match_metadata(root: &Path) -> Option<ManagedMediaMatchMetadata> {
    let contents = fs::read_to_string(managed_media_match_metadata_path(root)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn load_media_match_cache(root: &Path) -> Option<MediaMatchCacheV1> {
    let contents = fs::read_to_string(managed_media_match_cache_path(root)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn save_media_match_cache(root: &Path, cache: &MediaMatchCacheV1) -> Result<(), String> {
    let path = managed_media_match_cache_path(root);
    let contents = serde_json::to_string_pretty(cache)
        .map_err(|error| format!("failed serializing media-match cache: {error}"))?;
    fs::write(&path, contents).map_err(|error| {
        format!(
            "failed writing media-match cache '{}': {error}",
            path.display()
        )
    })
}

fn save_managed_media_match_metadata(
    root: &Path,
    metadata: &ManagedMediaMatchMetadata,
) -> Result<(), String> {
    let path = managed_media_match_metadata_path(root);
    let contents = serde_json::to_string_pretty(metadata)
        .map_err(|error| format!("failed serializing media-match metadata: {error}"))?;
    fs::write(&path, contents).map_err(|error| {
        format!(
            "failed writing media-match metadata '{}': {error}",
            path.display()
        )
    })
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(windows)]
fn media_match_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(MEDIA_MATCH_DOWNLOAD_TIMEOUT)
        .user_agent(MEDIA_MATCH_USER_AGENT)
        .build()
        .map_err(|error| format!("failed creating Media Matching HTTP client: {error}"))
}

#[cfg(windows)]
fn download_bytes_with_progress<F>(
    client: &reqwest::blocking::Client,
    url: &str,
    label: &str,
    progress_start: f32,
    progress_end: f32,
    progress: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    let mut response = client
        .get(url)
        .send()
        .map_err(|error| format!("failed downloading {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("failed downloading {url}: {error}"))?;
    let total_bytes = response.content_length();
    let capacity = total_bytes
        .and_then(|total| usize::try_from(total).ok())
        .map(|total| total.min(MEDIA_MATCH_DOWNLOAD_PREALLOC_MAX_BYTES))
        .unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    let mut buffer = [0u8; MEDIA_MATCH_DOWNLOAD_BUFFER_BYTES];
    let mut downloaded_bytes = 0u64;
    let mut next_progress_report = 0u64;

    loop {
        let read = response.read(&mut buffer).map_err(|error| {
            format!(
                "failed reading {url} after {}: {error}",
                format_downloaded_bytes(total_bytes, downloaded_bytes)
            )
        })?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        downloaded_bytes = downloaded_bytes.saturating_add(read as u64);
        if downloaded_bytes >= next_progress_report
            || total_bytes.is_some_and(|total| downloaded_bytes >= total)
        {
            progress(MediaMatchToolProgress::new(
                label,
                Some(download_progress_detail(url, total_bytes, downloaded_bytes)),
                download_progress_fraction(
                    progress_start,
                    progress_end,
                    total_bytes,
                    downloaded_bytes,
                ),
            ));
            next_progress_report =
                downloaded_bytes.saturating_add(MEDIA_MATCH_DOWNLOAD_PROGRESS_STEP_BYTES);
        }
    }

    if let Some(total) = total_bytes
        && downloaded_bytes < total
    {
        return Err(format!(
            "download from {url} ended early after {}",
            format_downloaded_bytes(total_bytes, downloaded_bytes)
        ));
    }
    progress(MediaMatchToolProgress::new(
        label,
        Some(download_progress_detail(url, total_bytes, downloaded_bytes)),
        progress_end,
    ));
    Ok(bytes)
}

#[cfg(any(windows, test))]
fn download_progress_fraction(
    progress_start: f32,
    progress_end: f32,
    total_bytes: Option<u64>,
    downloaded_bytes: u64,
) -> f32 {
    let span = (progress_end - progress_start).max(0.0);
    let fraction = total_bytes
        .filter(|total| *total > 0)
        .map(|total| (downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    (progress_start + (span * fraction)).clamp(progress_start, progress_end)
}

#[cfg(any(windows, test))]
fn download_progress_detail(url: &str, total_bytes: Option<u64>, downloaded_bytes: u64) -> String {
    format!(
        "{}: {}",
        download_source_label(url),
        format_downloaded_bytes(total_bytes, downloaded_bytes)
    )
}

#[cfg(any(windows, test))]
fn download_source_label(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .filter(|host| !host.is_empty())
        .unwrap_or(url)
        .to_owned()
}

#[cfg(any(windows, test))]
fn format_downloaded_bytes(total_bytes: Option<u64>, downloaded_bytes: u64) -> String {
    let downloaded = format_mib(downloaded_bytes);
    match total_bytes {
        Some(total) if total > 0 => format!("{downloaded} of {}", format_mib(total)),
        _ => downloaded,
    }
}

#[cfg(any(windows, test))]
fn format_mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
}

#[cfg(windows)]
fn extract_zip_entry(zip_bytes: &[u8], suffix: &str, target: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(reader)
        .map_err(|error| format!("failed reading downloaded zip archive: {error}"))?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| format!("failed reading zip entry {index}: {error}"))?;
        let name = file.name().replace('\\', "/");
        if !name.ends_with(suffix) {
            continue;
        }
        let mut output = fs::File::create(target)
            .map_err(|error| format!("failed creating '{}': {error}", target.display()))?;
        std::io::copy(&mut file, &mut output)
            .map_err(|error| format!("failed extracting '{}': {error}", target.display()))?;
        return Ok(());
    }
    Err(format!("downloaded archive did not contain {suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_media_match_test_root(label: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "sorotte-media-match-{label}-{}",
            std::process::id()
        ));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).expect("temp root should be created");
        root
    }

    #[test]
    fn media_match_clear_persisted_cache_removes_cache_and_tool_metadata() {
        let root = unique_media_match_test_root("clear");
        let metadata_dir = managed_media_match_bin_dir(&root);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        let cache_path = managed_media_match_cache_path(&root);
        let metadata_path = managed_media_match_metadata_path(&root);
        std::fs::write(&cache_path, r#"{"version":1,"records":{}}"#)
            .expect("cache should be written");
        std::fs::write(&metadata_path, r#"{"version":1}"#).expect("metadata should be written");

        clear_persisted_media_match_cache_at_root(&root).expect("clear should succeed");

        assert!(!cache_path.exists());
        assert!(!metadata_path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_download_progress_fraction_scales_between_bounds() {
        let progress = download_progress_fraction(0.08, 0.54, Some(100), 25);
        assert!((progress - 0.195).abs() < 0.0001);
        assert_eq!(download_progress_fraction(0.08, 0.54, Some(100), 150), 0.54);
    }

    #[test]
    fn media_match_download_progress_detail_uses_host_and_mib() {
        let detail = download_progress_detail(
            "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
            Some(2 * 1_048_576),
            1_048_576,
        );
        assert_eq!(detail, "www.gyan.dev: 1.0 MiB of 2.0 MiB");
    }
}
