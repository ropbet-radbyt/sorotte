use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use std::{io::Read, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sorotte_media_match::{
    MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFingerprintProfile,
    MediaFingerprintRecord, MediaMatchCacheV1, MediaMatchSettings, MediaMatchTier,
    MediaMatchToolPaths, decide_media_match, fingerprint_media_file_cancellable,
    media_match_wire_value_from_records, normalize_media_path, rank_media_match_candidates,
};

use super::shell_state::{
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchToolHealth,
    media_match_settings_from_stored_settings,
};

#[cfg(windows)]
use zip::ZipArchive;

const MEDIA_MATCH_METADATA_VERSION: u32 = 1;
const MEDIA_MATCH_CACHE_FILE: &str = "media-match-cache-v1.json";
const MEDIA_MATCH_INDEX_FILE: &str = "index-v2.sqlite3";
const MEDIA_MATCH_INDEX_BACKUP_FILE: &str = "index-v2.previous.sqlite3";
const MEDIA_MATCH_SQLITE_SCHEMA_VERSION: i64 = 1;
const MEDIA_MATCH_PREFILTER_THRESHOLD: usize = 64;
const MEDIA_MATCH_PREFILTER_LIMIT: usize = 24;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
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

pub(super) struct MediaMatchCandidateRebuildRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) candidates: Vec<PathBuf>,
    pub(super) current_player_path: Option<&'a str>,
    pub(super) settings: &'a MediaMatchSettings,
    pub(super) tools: &'a MediaMatchToolPaths,
    pub(super) extraction_settings: &'a MediaExtractionSettings,
    pub(super) cancel_flag: Option<&'a AtomicBool>,
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
    pub(super) fn display_name(self) -> &'static str {
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

fn managed_media_match_index_dir(root: &Path) -> PathBuf {
    root.join("cache").join("media-match")
}

fn managed_media_match_index_path(root: &Path) -> PathBuf {
    managed_media_match_index_dir(root).join(MEDIA_MATCH_INDEX_FILE)
}

fn managed_media_match_index_backup_path(root: &Path) -> PathBuf {
    managed_media_match_index_dir(root).join(MEDIA_MATCH_INDEX_BACKUP_FILE)
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text = path.as_os_str().to_os_string();
    text.push(suffix);
    PathBuf::from(text)
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("failed removing '{}': {error}", path.display()))?;
    }
    Ok(())
}

fn remove_sqlite_file_set(path: &Path) -> Result<(), String> {
    remove_file_if_exists(path)?;
    remove_file_if_exists(&path_with_suffix(path, "-wal"))?;
    remove_file_if_exists(&path_with_suffix(path, "-shm"))?;
    remove_file_if_exists(&path_with_suffix(path, "-journal"))?;
    Ok(())
}

pub(super) fn media_match_sqlite_index_exists(root: &Path) -> bool {
    managed_media_match_index_path(root).exists()
}

pub(super) fn prepare_media_match_index_rebuild_backup(root: &Path) -> Result<bool, String> {
    let index_dir = managed_media_match_index_dir(root);
    fs::create_dir_all(&index_dir).map_err(|error| {
        format!(
            "failed creating media-match index directory '{}': {error}",
            index_dir.display()
        )
    })?;
    let index_path = managed_media_match_index_path(root);
    let backup_path = managed_media_match_index_backup_path(root);
    remove_file_if_exists(&backup_path)?;
    if !index_path.exists() {
        return Ok(false);
    }
    fs::copy(&index_path, &backup_path).map_err(|error| {
        format!(
            "failed preserving previous media-match index '{}' as '{}': {error}",
            index_path.display(),
            backup_path.display()
        )
    })?;
    Ok(true)
}

pub(super) fn restore_media_match_index_rebuild_backup(
    root: &Path,
    backup_existed: bool,
) -> Result<(), String> {
    let index_path = managed_media_match_index_path(root);
    let backup_path = managed_media_match_index_backup_path(root);
    remove_sqlite_file_set(&index_path)?;
    if backup_existed {
        if !backup_path.exists() {
            return Err(format!(
                "previous media-match index backup '{}' is missing",
                backup_path.display()
            ));
        }
        fs::rename(&backup_path, &index_path).map_err(|error| {
            format!(
                "failed restoring previous media-match index '{}' to '{}': {error}",
                backup_path.display(),
                index_path.display()
            )
        })?;
    } else {
        remove_file_if_exists(&backup_path)?;
    }
    Ok(())
}

pub(super) fn discard_media_match_index_rebuild_backup(root: &Path) -> Result<(), String> {
    remove_file_if_exists(&managed_media_match_index_backup_path(root))
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
    let index_path = managed_media_match_index_path(root);
    remove_sqlite_file_set(&index_path)
        .map_err(|error| format!("failed removing media-match SQLite index: {error}"))?;
    remove_file_if_exists(&managed_media_match_index_backup_path(root))?;
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
        remote_status: Some("unavailable".to_owned()),
        background_status: Some("idle".to_owned()),
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

pub(super) fn rebuild_persisted_media_match_index_with_progress_and_cancel<F>(
    root: &Path,
    search_roots: &[PathBuf],
    current_player_path: Option<&str>,
    settings: &MediaMatchSettings,
    cancel_flag: Option<&AtomicBool>,
    progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    let extraction_settings = MediaExtractionSettings::fast_v1();
    rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        root,
        search_roots,
        current_player_path,
        settings,
        &extraction_settings,
        cancel_flag,
        progress,
    )
}

pub(super) fn rebuild_persisted_media_match_index_with_extraction_settings_and_cancel<F>(
    root: &Path,
    search_roots: &[PathBuf],
    current_player_path: Option<&str>,
    settings: &MediaMatchSettings,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
    mut progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    if !settings.fingerprinting_enabled {
        return Err("Enable Media Matching fingerprinting before rebuilding the index.".to_owned());
    }
    let tools = media_match_tool_paths(root)?;
    progress(MediaMatchToolProgress::new(
        "Scanning media-search roots",
        Some(format!("{} roots", search_roots.len())),
        0.05,
    ));
    let candidates = collect_media_match_candidates(search_roots);
    rebuild_persisted_media_match_candidates_with_progress_and_cancel(
        MediaMatchCandidateRebuildRequest {
            root,
            candidates,
            current_player_path,
            settings,
            tools: &tools,
            extraction_settings,
            cancel_flag,
        },
        progress,
    )
}

pub(super) fn rebuild_persisted_media_match_candidates_with_progress_and_cancel<F>(
    request: MediaMatchCandidateRebuildRequest<'_>,
    mut progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    let selected =
        select_media_match_rebuild_candidates(&request.candidates, request.current_player_path);
    let existing_cache =
        load_media_match_cache_for_settings(request.root, request.extraction_settings)
            .unwrap_or_default();
    let checkpoint_connection = open_media_match_sqlite_index(request.root)?;
    let mut next_cache = initial_media_match_rebuild_cache(&existing_cache, selected.prefiltered);
    let mut reused = 0usize;
    let mut fingerprinted = 0usize;
    let mut skipped = 0usize;
    let total = selected.paths.len();
    let fresh_work_total = selected
        .paths
        .iter()
        .filter(|path| {
            !media_match_cache_has_valid_record(&existing_cache, path, request.extraction_settings)
        })
        .count();
    let mut fresh_work_done = 0usize;
    let mut query_record = None;
    let normalized_current_path = request.current_player_path.map(normalize_media_path);
    let mut strong_match_found = false;
    progress(MediaMatchToolProgress::new(
        "Fingerprinting media",
        Some(format!("0/{fresh_work_total} files needing index")),
        0.1,
    ));

    for (index, path) in selected.paths.iter().enumerate() {
        if request
            .cancel_flag
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err("Media Matching index rebuild was canceled.".to_owned());
        }
        let denominator = total.max(1);
        let progress_fraction = 0.1 + (0.82 * (index as f32 / denominator as f32));
        let path_needs_fingerprint =
            !media_match_cache_has_valid_record(&existing_cache, path, request.extraction_settings);
        progress(MediaMatchToolProgress::new(
            "Fingerprinting media",
            Some(format!(
                "{fresh_work_done}/{fresh_work_total} files needing index: {}",
                path.display()
            )),
            progress_fraction,
        ));
        match cached_or_fresh_media_fingerprint(
            &existing_cache,
            path,
            request.tools,
            request.extraction_settings,
            request.cancel_flag,
        ) {
            Ok((record, was_reused)) => {
                if was_reused {
                    reused += 1;
                } else {
                    fingerprinted += 1;
                    fresh_work_done += 1;
                }
                if normalized_current_path.as_deref()
                    == Some(record.identity.normalized_path.as_str())
                {
                    query_record = Some(record.clone());
                } else if let Some(query) = query_record.as_ref() {
                    let decision = decide_media_match(query, &record, request.settings);
                    strong_match_found = decision.tier == MediaMatchTier::Strong;
                }
                save_media_match_record_to_sqlite(&checkpoint_connection, &record)?;
                next_cache.insert(record);
                if strong_match_found {
                    break;
                }
            }
            Err(_) => {
                skipped += 1;
                if path_needs_fingerprint {
                    fresh_work_done += 1;
                }
            }
        }
    }

    let (current_decision, last_evidence) =
        summarize_current_media_match(request.current_player_path, &next_cache, request.settings);
    let cache_status = format!("{} fingerprint records", next_cache.records.len());
    let attempted = reused + fingerprinted + skipped;
    let scope = if selected.prefiltered {
        format!(
            "{} of {} discovered files",
            attempted, selected.discovered_files
        )
    } else {
        format!("{} files", selected.discovered_files)
    };
    let message = format!(
        "Media Matching indexed {scope} ({} reused, {} fingerprinted, {} skipped).",
        reused, fingerprinted, skipped
    );
    progress(MediaMatchToolProgress::new(
        "Media Matching index rebuilt",
        Some(format!(
            "{fresh_work_done}/{fresh_work_total} files needing index; {cache_status}"
        )),
        1.0,
    ));

    Ok(MediaMatchIndexRebuildResult {
        message,
        cache_status,
        current_decision,
        last_evidence,
    })
}

fn initial_media_match_rebuild_cache(
    existing_cache: &MediaMatchCacheV1,
    prefiltered: bool,
) -> MediaMatchCacheV1 {
    if prefiltered {
        existing_cache.clone()
    } else {
        MediaMatchCacheV1::default()
    }
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
    match media_match_sqlite_counts(root) {
        Ok((inventory, fast, full)) if inventory > 0 || fast > 0 || full > 0 => {
            format!("inventory: {inventory}, fast: {fast}, full: {full}")
        }
        Ok(_) => {
            let path = managed_media_match_cache_path(root);
            let Ok(contents) = fs::read_to_string(&path) else {
                return "empty".to_owned();
            };
            match serde_json::from_str::<MediaMatchCacheV1>(&contents) {
                Ok(cache) => format!(
                    "inventory: {}, fast: 0, full: {}",
                    cache.records.len(),
                    cache.records.len()
                ),
                Err(error) => format!("unreadable cache: {error}"),
            }
        }
        Err(error) => format!("unreadable cache: {error}"),
    }
}

pub(super) fn media_match_tool_paths(root: &Path) -> Result<MediaMatchToolPaths, String> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaMatchRebuildCandidateSelection {
    paths: Vec<PathBuf>,
    discovered_files: usize,
    prefiltered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaMatchFilenameProfile {
    series_tokens: BTreeSet<String>,
    season: Option<u32>,
    episode: Option<u32>,
    numbers: BTreeSet<u32>,
}

fn select_media_match_rebuild_candidates(
    candidates: &[PathBuf],
    current_player_path: Option<&str>,
) -> MediaMatchRebuildCandidateSelection {
    let normalized_current_path = current_player_path.map(normalize_media_path);
    let current_path = current_player_path
        .map(PathBuf::from)
        .filter(|path| path.is_file() && media_match_candidate_extension(path));
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(path) = current_path {
        seen.insert(normalize_media_path(&path));
        paths.push(path);
    }

    let root_candidates = candidates
        .iter()
        .filter(|path| {
            normalized_current_path
                .as_deref()
                .is_none_or(|current| normalize_media_path(path) != current)
        })
        .cloned()
        .collect::<Vec<_>>();

    let prefiltered = normalized_current_path.is_some()
        && root_candidates.len() > MEDIA_MATCH_PREFILTER_THRESHOLD;
    let selected_root_candidates = if prefiltered {
        prefilter_media_match_candidates(
            &root_candidates,
            current_player_path.expect("prefilter requires a current path"),
        )
    } else {
        root_candidates
    };

    for path in selected_root_candidates {
        if seen.insert(normalize_media_path(&path)) {
            paths.push(path);
        }
    }

    MediaMatchRebuildCandidateSelection {
        paths,
        discovered_files: candidates.len(),
        prefiltered,
    }
}

fn prefilter_media_match_candidates(
    candidates: &[PathBuf],
    current_player_path: &str,
) -> Vec<PathBuf> {
    let query = media_match_filename_profile(Path::new(current_player_path));
    let mut scored = candidates
        .iter()
        .map(|path| {
            (
                media_match_filename_score(&query, &media_match_filename_profile(path)),
                path.clone(),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let mut selected = scored
        .iter()
        .filter(|(score, _)| *score > 0)
        .take(MEDIA_MATCH_PREFILTER_LIMIT)
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        selected = scored
            .into_iter()
            .take(MEDIA_MATCH_PREFILTER_LIMIT)
            .map(|(_, path)| path)
            .collect();
    }
    selected
}

fn media_match_filename_profile(path: &Path) -> MediaMatchFilenameProfile {
    let tokens = media_match_filename_tokens(path);
    let mut series_tokens = BTreeSet::new();
    let mut numbers = BTreeSet::new();
    let mut season = None;
    let mut episode = None;

    for (index, token) in tokens.iter().enumerate() {
        if let Some((parsed_season, parsed_episode)) = parse_season_episode_token(token) {
            season.get_or_insert(parsed_season);
            episode.get_or_insert(parsed_episode);
            continue;
        }
        if token == "season" {
            if index > 0
                && let Some(parsed_season) = parse_ordinal_or_number(&tokens[index - 1])
            {
                season.get_or_insert(parsed_season);
            }
            if episode.is_none() {
                episode = tokens
                    .iter()
                    .skip(index + 1)
                    .find_map(|candidate| parse_ordinal_or_number(candidate));
            }
            continue;
        }
        if let Some(number) = parse_ordinal_or_number(token) {
            numbers.insert(number);
            continue;
        }
        if is_media_match_filename_noise_token(token) {
            continue;
        }
        series_tokens.insert(token.clone());
    }

    MediaMatchFilenameProfile {
        series_tokens,
        season,
        episode,
        numbers,
    }
}

fn media_match_filename_score(
    query: &MediaMatchFilenameProfile,
    candidate: &MediaMatchFilenameProfile,
) -> i32 {
    let mut score = 0;
    if query.season.is_some() && query.season == candidate.season {
        score += 50;
    }
    if query.episode.is_some() && query.episode == candidate.episode {
        score += 120;
    } else if query
        .episode
        .is_some_and(|episode| candidate.numbers.contains(&episode))
    {
        score += 40;
    }
    if query.season.is_some()
        && query.season == candidate.season
        && query.episode.is_some()
        && query.episode == candidate.episode
    {
        score += 80;
    }
    let overlap = query
        .series_tokens
        .intersection(&candidate.series_tokens)
        .count();
    if !query.series_tokens.is_empty() {
        score += ((overlap * 100) / query.series_tokens.len()) as i32;
        if overlap == 0 {
            score -= 40;
        }
    }
    score
}

fn media_match_filename_tokens(path: &Path) -> Vec<String> {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut without_groups = String::new();
    let mut square_depth = 0u32;
    for character in stem.chars() {
        match character {
            '[' => square_depth = square_depth.saturating_add(1),
            ']' => square_depth = square_depth.saturating_sub(1),
            _ if square_depth > 0 => {}
            _ if character.is_ascii_alphanumeric() => {
                without_groups.push(character.to_ascii_lowercase());
            }
            _ => without_groups.push(' '),
        }
    }
    without_groups
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn parse_season_episode_token(token: &str) -> Option<(u32, u32)> {
    let lower = token.to_ascii_lowercase();
    let rest = lower.strip_prefix('s')?;
    let (season_text, episode_text) = rest.split_once('e')?;
    let season = season_text.parse::<u32>().ok()?;
    let episode = episode_text.parse::<u32>().ok()?;
    Some((season, episode))
}

fn parse_ordinal_or_number(token: &str) -> Option<u32> {
    let trimmed = token
        .strip_suffix("st")
        .or_else(|| token.strip_suffix("nd"))
        .or_else(|| token.strip_suffix("rd"))
        .or_else(|| token.strip_suffix("th"))
        .unwrap_or(token);
    if trimmed.len() > 3
        || trimmed.is_empty()
        || !trimmed.chars().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    trimmed.parse::<u32>().ok()
}

fn is_media_match_filename_noise_token(token: &str) -> bool {
    matches!(
        token,
        "bd" | "bdrip"
            | "webrip"
            | "web"
            | "dl"
            | "hevc"
            | "x264"
            | "x265"
            | "h264"
            | "h265"
            | "aac"
            | "flac"
            | "dual"
            | "audio"
            | "multi"
            | "multisub"
            | "sub"
            | "subs"
            | "raws"
    ) || token.ends_with('p')
        && token[..token.len() - 1]
            .chars()
            .all(|character| character.is_ascii_digit())
        || token.len() == 8 && token.chars().all(|character| character.is_ascii_hexdigit())
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
    cancel_flag: Option<&AtomicBool>,
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
    fingerprint_media_file_cancellable(
        path,
        tools,
        extraction_settings,
        cancel_flag.unwrap_or(&AtomicBool::new(false)),
    )
    .map(|record| (record, false))
    .map_err(|error| error.to_string())
}

fn media_match_cache_has_valid_record(
    existing_cache: &MediaMatchCacheV1,
    path: &Path,
    extraction_settings: &MediaExtractionSettings,
) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    existing_cache
        .get_valid(
            path,
            modified_unix_millis,
            metadata.len(),
            MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings,
        )
        .is_some()
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

pub(super) fn media_match_tier_label(tier: MediaMatchTier) -> &'static str {
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
    let output = hidden_media_match_command(path)
        .args(args)
        .output()
        .map_err(|error| {
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

#[cfg(windows)]
fn hidden_media_match_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn hidden_media_match_command(path: &Path) -> Command {
    Command::new(path)
}

fn load_managed_media_match_metadata(root: &Path) -> Option<ManagedMediaMatchMetadata> {
    let contents = fs::read_to_string(managed_media_match_metadata_path(root)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn load_media_match_json_cache(root: &Path) -> Option<MediaMatchCacheV1> {
    let contents = fs::read_to_string(managed_media_match_cache_path(root)).ok()?;
    serde_json::from_str(&contents).ok()
}

fn open_media_match_sqlite_index(root: &Path) -> Result<Connection, String> {
    let index_dir = managed_media_match_index_dir(root);
    fs::create_dir_all(&index_dir).map_err(|error| {
        format!(
            "failed creating media-match index directory '{}': {error}",
            index_dir.display()
        )
    })?;
    let path = managed_media_match_index_path(root);
    let connection = Connection::open(&path).map_err(|error| {
        format!(
            "failed opening media-match SQLite index '{}': {error}",
            path.display()
        )
    })?;
    initialize_media_match_sqlite_index(&connection)?;
    migrate_media_match_json_cache_to_sqlite(root, &connection)?;
    Ok(connection)
}

fn initialize_media_match_sqlite_index(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA user_version = 1;
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS fingerprints (
                normalized_path TEXT NOT NULL,
                profile TEXT NOT NULL,
                modified_unix_millis INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                algorithm_version INTEGER NOT NULL,
                extraction_settings_json TEXT NOT NULL,
                duration_seconds REAL,
                record_json TEXT NOT NULL,
                last_error TEXT,
                updated_unix_millis INTEGER NOT NULL,
                PRIMARY KEY (normalized_path, profile)
            );
            CREATE INDEX IF NOT EXISTS idx_media_match_fingerprints_profile
                ON fingerprints(profile);
            ",
        )
        .map_err(|error| format!("failed initializing media-match SQLite index: {error}"))?;
    connection
        .pragma_update(None, "user_version", MEDIA_MATCH_SQLITE_SCHEMA_VERSION)
        .map_err(|error| format!("failed setting media-match SQLite schema version: {error}"))?;
    Ok(())
}

fn migrate_media_match_json_cache_to_sqlite(
    root: &Path,
    connection: &Connection,
) -> Result<(), String> {
    let migrated = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'json_v1_migrated'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("failed checking media-match JSON migration state: {error}"))?;
    if migrated.as_deref() == Some("true") {
        return Ok(());
    }
    if let Some(cache) = load_media_match_json_cache(root) {
        save_media_match_cache_to_sqlite(connection, &cache)?;
    }
    connection
        .execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('json_v1_migrated', 'true')",
            [],
        )
        .map_err(|error| format!("failed recording media-match JSON migration state: {error}"))?;
    Ok(())
}

fn media_match_profile_label(settings: &MediaExtractionSettings) -> &'static str {
    settings.profile.label()
}

pub(super) fn load_media_match_cache_for_settings(
    root: &Path,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchCacheV1> {
    let connection = open_media_match_sqlite_index(root).ok()?;
    let mut statement = connection
        .prepare(
            "SELECT record_json FROM fingerprints
             WHERE profile = ?1 AND algorithm_version = ?2",
        )
        .ok()?;
    let rows = statement
        .query_map(
            params![
                media_match_profile_label(extraction_settings),
                i64::from(MEDIA_MATCH_ALGORITHM_VERSION),
            ],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    let mut cache = MediaMatchCacheV1::default();
    for record_json in rows.flatten() {
        if let Ok(record) = serde_json::from_str::<MediaFingerprintRecord>(&record_json) {
            cache.insert(record);
        }
    }
    Some(cache)
}

pub(super) fn media_match_wire_value_for_path(
    root: &Path,
    current_player_path: &str,
) -> Option<serde_json::Value> {
    let fast_record = media_match_record_for_path(
        root,
        current_player_path,
        &MediaExtractionSettings::fast_v1(),
    )?;
    let mut records = vec![fast_record.clone()];
    if let Some(full_record) = media_match_record_for_path(
        root,
        current_player_path,
        &MediaExtractionSettings::full_v1(),
    ) {
        records.push(full_record);
    }
    media_match_wire_value_from_records(&records)
        .or_else(|| media_match_wire_value_from_records(std::slice::from_ref(&fast_record)))
}

pub(super) fn media_match_record_for_path(
    root: &Path,
    current_player_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaFingerprintRecord> {
    let normalized_path = normalize_media_path(current_player_path);
    let metadata = fs::metadata(current_player_path).ok()?;
    let modified_unix_millis = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    let size_bytes = metadata.len();
    load_media_match_cache_for_settings(root, extraction_settings)
        .and_then(|cache| cache.records.get(&normalized_path).cloned())
        .filter(|record| {
            record.valid_for(
                &normalized_path,
                modified_unix_millis,
                size_bytes,
                MEDIA_MATCH_ALGORITHM_VERSION,
                extraction_settings,
            )
        })
}

#[cfg(test)]
fn save_media_match_cache(root: &Path, cache: &MediaMatchCacheV1) -> Result<(), String> {
    let connection = open_media_match_sqlite_index(root)?;
    save_media_match_cache_to_sqlite(&connection, cache)
}

fn save_media_match_cache_to_sqlite(
    connection: &Connection,
    cache: &MediaMatchCacheV1,
) -> Result<(), String> {
    let now = current_unix_millis();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match SQLite transaction: {error}"))?;
    for record in cache.records.values() {
        let extraction_settings_json =
            serde_json::to_string(&record.extraction_settings).map_err(|error| {
                format!("failed serializing media-match extraction settings: {error}")
            })?;
        let record_json = serde_json::to_string(record)
            .map_err(|error| format!("failed serializing media-match record: {error}"))?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO fingerprints (
                    normalized_path,
                    profile,
                    modified_unix_millis,
                    size_bytes,
                    algorithm_version,
                    extraction_settings_json,
                    duration_seconds,
                    record_json,
                    last_error,
                    updated_unix_millis
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
                params![
                    record.identity.normalized_path,
                    record.extraction_settings.profile.label(),
                    record.identity.modified_unix_millis as i64,
                    record.identity.size_bytes as i64,
                    record.algorithm_version as i64,
                    extraction_settings_json,
                    record.duration_seconds,
                    record_json,
                    now as i64,
                ],
            )
            .map_err(|error| format!("failed writing media-match fingerprint record: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("failed committing media-match SQLite transaction: {error}"))
}

fn save_media_match_record_to_sqlite(
    connection: &Connection,
    record: &MediaFingerprintRecord,
) -> Result<(), String> {
    let extraction_settings_json = serde_json::to_string(&record.extraction_settings)
        .map_err(|error| format!("failed serializing media-match extraction settings: {error}"))?;
    let record_json = serde_json::to_string(record)
        .map_err(|error| format!("failed serializing media-match record: {error}"))?;
    connection
        .execute(
            "INSERT OR REPLACE INTO fingerprints (
                normalized_path,
                profile,
                modified_unix_millis,
                size_bytes,
                algorithm_version,
                extraction_settings_json,
                duration_seconds,
                record_json,
                last_error,
                updated_unix_millis
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
            params![
                record.identity.normalized_path,
                record.extraction_settings.profile.label(),
                record.identity.modified_unix_millis as i64,
                record.identity.size_bytes as i64,
                record.algorithm_version as i64,
                extraction_settings_json,
                record.duration_seconds,
                record_json,
                current_unix_millis() as i64,
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("failed checkpointing media-match fingerprint record: {error}"))
}

fn media_match_sqlite_counts(root: &Path) -> Result<(usize, usize, usize), String> {
    if !managed_media_match_index_path(root).exists() {
        return Ok((0, 0, 0));
    }
    let connection = open_media_match_sqlite_index(root)?;
    let inventory = connection
        .query_row(
            "SELECT COUNT(DISTINCT normalized_path) FROM fingerprints",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed reading media-match inventory count: {error}"))?
        .max(0) as usize;
    let count_for_profile = |profile: &str| -> Result<usize, String> {
        Ok(connection
            .query_row(
                "SELECT COUNT(*) FROM fingerprints WHERE profile = ?1",
                [profile],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("failed reading media-match profile count: {error}"))?
            .max(0) as usize)
    };
    Ok((
        inventory,
        count_for_profile(MediaFingerprintProfile::FastV1.label())?,
        count_for_profile(MediaFingerprintProfile::FullV1.label())?,
    ))
}

#[allow(dead_code)]
fn save_media_match_json_cache(root: &Path, cache: &MediaMatchCacheV1) -> Result<(), String> {
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

fn current_unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(u128::from(u64::MAX)) as u64
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

    fn fake_media_match_record(path: &str) -> MediaFingerprintRecord {
        MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(path, 1000, 2000),
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: MediaExtractionSettings::default(),
            duration_seconds: Some(1200.0),
            container_fingerprint: format!("container:{path}"),
            audio: None,
            video: None,
        }
    }

    #[test]
    fn media_match_clear_persisted_cache_removes_cache_and_tool_metadata() {
        let root = unique_media_match_test_root("clear");
        let metadata_dir = managed_media_match_bin_dir(&root);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        let cache_path = managed_media_match_cache_path(&root);
        let index_path = managed_media_match_index_path(&root);
        let metadata_path = managed_media_match_metadata_path(&root);
        std::fs::write(&cache_path, r#"{"version":1,"records":{}}"#)
            .expect("cache should be written");
        let mut sqlite_cache = MediaMatchCacheV1::default();
        sqlite_cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_cache(&root, &sqlite_cache).expect("SQLite cache should be written");
        assert!(index_path.exists());
        std::fs::write(&metadata_path, r#"{"version":1}"#).expect("metadata should be written");

        clear_persisted_media_match_cache_at_root(&root).expect("clear should succeed");

        assert!(!cache_path.exists());
        assert!(!index_path.exists());
        assert!(!metadata_path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_index_backup_restore_reinstates_previous_sqlite_index() {
        let root = unique_media_match_test_root("restore");
        let mut previous_cache = MediaMatchCacheV1::default();
        previous_cache.insert(fake_media_match_record("previous.mkv"));
        save_media_match_cache(&root, &previous_cache).expect("previous cache should be written");

        let backup_existed =
            prepare_media_match_index_rebuild_backup(&root).expect("backup should be prepared");
        assert!(backup_existed);

        remove_sqlite_file_set(&managed_media_match_index_path(&root))
            .expect("primary index should be removed for test");
        let mut partial_cache = MediaMatchCacheV1::default();
        partial_cache.insert(fake_media_match_record("partial.mkv"));
        save_media_match_cache(&root, &partial_cache).expect("partial cache should be written");

        restore_media_match_index_rebuild_backup(&root, backup_existed)
            .expect("previous index should be restored");
        let restored =
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::default())
                .expect("restored cache should load");

        assert!(
            restored
                .records
                .contains_key(&normalize_media_path("previous.mkv"))
        );
        assert!(
            !restored
                .records
                .contains_key(&normalize_media_path("partial.mkv"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_index_backup_restore_removes_partial_index_without_previous_db() {
        let root = unique_media_match_test_root("restore-empty");
        let backup_existed =
            prepare_media_match_index_rebuild_backup(&root).expect("backup should be prepared");
        assert!(!backup_existed);

        let mut partial_cache = MediaMatchCacheV1::default();
        partial_cache.insert(fake_media_match_record("partial.mkv"));
        save_media_match_cache(&root, &partial_cache).expect("partial cache should be written");
        assert!(managed_media_match_index_path(&root).exists());

        restore_media_match_index_rebuild_backup(&root, backup_existed)
            .expect("partial index should be removed");
        assert!(!managed_media_match_index_path(&root).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_sqlite_index_tracks_fast_and_full_profiles_separately() {
        let root = unique_media_match_test_root("sqlite-profiles");
        let mut full_cache = MediaMatchCacheV1::default();
        full_cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_cache(&root, &full_cache).expect("full cache should be written");

        let mut fast_record = fake_media_match_record("episode.mkv");
        fast_record.extraction_settings = MediaExtractionSettings::fast_v1();
        let mut fast_cache = MediaMatchCacheV1::default();
        fast_cache.insert(fast_record);
        save_media_match_cache(&root, &fast_cache).expect("fast cache should be written");

        assert_eq!(
            media_match_sqlite_counts(&root).expect("counts should load"),
            (1, 1, 1)
        );
        assert_eq!(
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::fast_v1())
                .expect("fast cache should load")
                .records
                .len(),
            1
        );
        assert_eq!(
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::full_v1())
                .expect("full cache should load")
                .records
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_json_v1_cache_migrates_to_sqlite_v2() {
        let root = unique_media_match_test_root("json-migration");
        let mut json_cache = MediaMatchCacheV1::default();
        json_cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_json_cache(&root, &json_cache).expect("JSON cache should be written");

        let migrated =
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::full_v1())
                .expect("migrated cache should load");

        assert_eq!(migrated.records.len(), 1);
        assert!(managed_media_match_index_path(&root).exists());
        assert_eq!(
            media_match_sqlite_counts(&root).expect("counts should load"),
            (1, 0, 1)
        );
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

    #[test]
    fn media_match_prefiltered_rebuild_preserves_existing_cache_records() {
        let mut existing = MediaMatchCacheV1::default();
        existing.insert(fake_media_match_record("E:/Anime/Re Zero/episode-01.mkv"));
        existing.insert(fake_media_match_record("E:/Anime/Re Zero/episode-02.mkv"));

        let prefiltered = initial_media_match_rebuild_cache(&existing, true);
        let full = initial_media_match_rebuild_cache(&existing, false);

        assert_eq!(prefiltered.records.len(), 2);
        assert!(full.records.is_empty());
    }

    #[test]
    fn media_match_filename_profile_matches_s04e07_to_4th_season_07() {
        let query = media_match_filename_profile(Path::new("[Judas] Re.Zero - S04E07.mkv"));
        let matching_candidate = media_match_filename_profile(Path::new(
            "[Erai-raws] Re Zero kara Hajimeru Isekai Seikatsu 4th Season - 07 [1080p CR WEBRip HEVC AAC][MultiSub][5CA89B15].mkv",
        ));
        let wrong_candidate = media_match_filename_profile(Path::new(
            "[Erai-raws] Medalist 2nd Season - 07 [1080p CR WEBRip HEVC AAC][MultiSub].mkv",
        ));

        let matching_score = media_match_filename_score(&query, &matching_candidate);
        let wrong_score = media_match_filename_score(&query, &wrong_candidate);

        assert_eq!(query.season, Some(4));
        assert_eq!(query.episode, Some(7));
        assert_eq!(matching_candidate.season, Some(4));
        assert_eq!(matching_candidate.episode, Some(7));
        assert!(matching_score > wrong_score);
        assert!(matching_score > 0);
    }

    #[test]
    fn media_match_rebuild_selection_prefilters_large_roots_for_current_file() {
        let root = unique_media_match_test_root("prefilter");
        let current_dir = root.join("downloads");
        std::fs::create_dir_all(&current_dir).expect("current dir should be created");
        let current_path = current_dir.join("[Judas] Re.Zero - S04E07.mkv");
        std::fs::write(&current_path, b"test")
            .expect("current media placeholder should be written");
        let matching_path = root.join("anime").join("Re Zero kara Hajimeru Isekai Seikatsu (2026)").join(
            "[Erai-raws] Re Zero kara Hajimeru Isekai Seikatsu 4th Season - 07 [1080p CR WEBRip HEVC AAC][MultiSub][5CA89B15].mkv",
        );
        let mut candidates = vec![matching_path.clone()];
        candidates.extend((0..MEDIA_MATCH_PREFILTER_THRESHOLD + 10).map(|index| {
            PathBuf::from(format!(
                "E:/Anime/Unrelated Series {index:03}/[Example] Other Show - {index:02}.mkv"
            ))
        }));

        let selection = select_media_match_rebuild_candidates(&candidates, current_path.to_str());

        assert!(selection.prefiltered);
        assert_eq!(selection.discovered_files, candidates.len());
        assert_eq!(selection.paths.first(), Some(&current_path));
        assert!(selection.paths.contains(&matching_path));
        assert!(selection.paths.len() <= MEDIA_MATCH_PREFILTER_LIMIT + 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_rebuild_selection_keeps_all_candidates_for_small_roots() {
        let root = unique_media_match_test_root("small-root-selection");
        let current_dir = root.join("downloads");
        std::fs::create_dir_all(&current_dir).expect("current dir should be created");
        let current_path = current_dir.join("[Judas] Re.Zero - S04E07.mkv");
        std::fs::write(&current_path, b"test")
            .expect("current media placeholder should be written");
        let candidates = vec![
            PathBuf::from("E:/Anime/Re Zero/[Erai-raws] Re Zero 4th Season - 07.mkv"),
            PathBuf::from("E:/Anime/Re Zero/[Erai-raws] Re Zero 4th Season - 08.mkv"),
        ];

        let selection = select_media_match_rebuild_candidates(&candidates, current_path.to_str());

        assert!(!selection.prefiltered);
        assert_eq!(selection.discovered_files, candidates.len());
        assert_eq!(selection.paths.first(), Some(&current_path));
        assert_eq!(selection.paths.len(), candidates.len() + 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
