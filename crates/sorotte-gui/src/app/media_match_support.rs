use std::{
    collections::{BTreeMap, BTreeSet},
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
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION, MEDIA_MATCH_ANCHOR_VERSION,
    MediaExtractionSettings, MediaFingerprintError, MediaFingerprintProfile,
    MediaFingerprintRecord, MediaMatchCache, MediaMatchCandidateDecision, MediaMatchDecision,
    MediaMatchSettings, MediaMatchTier, MediaMatchToolPaths, VideoAnchor,
    audio_anchors_from_record, decide_media_match, decode_audio_anchor_summary,
    decode_video_anchor_summary, fingerprint_media_file_cancellable_with_report,
    media_extraction_settings_hash, media_fingerprint_summary_from_record,
    media_match_wire_value_from_records, normalize_media_path, rank_media_match_candidates,
    video_anchor_hashes_match, video_anchors_from_record,
};

use super::shell_state::{
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchToolHealth,
    media_match_settings_from_stored_settings,
};

#[cfg(windows)]
use zip::ZipArchive;

const MEDIA_MATCH_METADATA_VERSION: u32 = 1;
const MEDIA_MATCH_INDEX_FILE: &str = "index-v2.sqlite3";
const MEDIA_MATCH_INDEX_BACKUP_FILE: &str = "index-v2.previous.sqlite3";
const MEDIA_MATCH_SQLITE_SCHEMA_VERSION: i64 = 2;
const MEDIA_MATCH_PREFILTER_THRESHOLD: usize = 64;
const MEDIA_MATCH_PREFILTER_LIMIT: usize = 24;
const MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MIN_CANDIDATES: usize = 4;
const MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_QUERY_ANCHORS: usize = 16;
const MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_ROWS_PER_QUERY: i64 = 50_000;
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
    pub(super) nearest_match: Option<String>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MediaMatchRebuildInstrumentation {
    ffmpeg_invocations: u32,
    ffprobe_invocations: u32,
    fpcalc_invocations: u32,
    extraction_millis: u128,
    ffprobe_millis: u128,
    audio_millis: u128,
    video_millis: u128,
    debug_record_bytes: usize,
    audio_summary_bytes: usize,
    video_summary_bytes: usize,
}

impl MediaMatchRebuildInstrumentation {
    fn add_report(&mut self, report: &sorotte_media_match::MediaFingerprintExtractionReport) {
        self.ffmpeg_invocations += report.invocations.ffmpeg;
        self.ffprobe_invocations += report.invocations.ffprobe;
        self.fpcalc_invocations += report.invocations.fpcalc;
        self.extraction_millis += report.timings.total_millis;
        self.ffprobe_millis += report.timings.ffprobe_millis;
        self.audio_millis += report.timings.audio_millis;
        self.video_millis += report.timings.video_millis;
        self.debug_record_bytes += report.serialized_debug_record_bytes;
        self.audio_summary_bytes += report.audio_summary_bytes;
        self.video_summary_bytes += report.video_summary_bytes;
    }

    fn summary(&self) -> String {
        format!(
            "tools ffmpeg/ffprobe/fpcalc={}/{}/{}, extract={}ms (probe {}ms, audio {}ms, video {}ms), v2 summary bytes audio/video={}/{} (debug record bytes={})",
            self.ffmpeg_invocations,
            self.ffprobe_invocations,
            self.fpcalc_invocations,
            self.extraction_millis,
            self.ffprobe_millis,
            self.audio_millis,
            self.video_millis,
            self.audio_summary_bytes,
            self.video_summary_bytes,
            self.debug_record_bytes
        )
    }
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
        nearest_match: None,
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
    progress(MediaMatchToolProgress::new(
        "Scanning media-search roots",
        Some(format!("{} roots", search_roots.len())),
        0.05,
    ));
    let candidates = collect_media_match_candidates(search_roots);
    if current_player_path.is_none() {
        inventory_media_match_candidates(root, search_roots, &candidates, cancel_flag)?;
        let cache_status = media_match_cache_status(root);
        progress(MediaMatchToolProgress::new(
            "Media Matching inventory updated",
            Some(cache_status.clone()),
            1.0,
        ));
        return Ok(MediaMatchIndexRebuildResult {
            message: format!(
                "Media Matching inventoried {} discovered files. No active local media path could be resolved, so fingerprinting is idle until the player or selected playlist item resolves to a local file.",
                candidates.len()
            ),
            cache_status,
            current_decision: Some("unknown: no resolved current local file".to_owned()),
            nearest_match: None,
            last_evidence: None,
        });
    }
    let tools = media_match_tool_paths(root)?;
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
    if fresh_work_total == 0 {
        let (current_decision, nearest_match, last_evidence) = summarize_current_media_match(
            request.root,
            request.current_player_path,
            &existing_cache,
            request.settings,
            request.extraction_settings,
        );
        let cache_status = media_match_cache_status(request.root);
        progress(MediaMatchToolProgress::new(
            "Media Matching index rebuilt",
            Some(format!("0/0 files needing index; {cache_status}")),
            1.0,
        ));
        return Ok(MediaMatchIndexRebuildResult {
            message: format!(
                "Media Matching index already current ({} discovered files, 0 needing index).",
                selected.discovered_files
            ),
            cache_status,
            current_decision,
            nearest_match,
            last_evidence,
        });
    }

    let checkpoint_connection = open_media_match_sqlite_index(request.root)?;
    let mut next_cache = initial_media_match_rebuild_cache(&existing_cache, selected.prefiltered);
    let mut instrumentation = MediaMatchRebuildInstrumentation::default();

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
            Ok((record, was_reused, report)) => {
                if was_reused {
                    reused += 1;
                } else {
                    fingerprinted += 1;
                    fresh_work_done += 1;
                    save_media_match_record_to_sqlite(&checkpoint_connection, &record)?;
                    if let Some(report) = report {
                        instrumentation.add_report(&report);
                    }
                }
                if normalized_current_path.as_deref()
                    == Some(record.identity.normalized_path.as_str())
                {
                    query_record = Some(record.clone());
                } else if let Some(query) = query_record.as_ref() {
                    let decision = decide_media_match(query, &record, request.settings);
                    strong_match_found = decision.tier == MediaMatchTier::Strong;
                }
                next_cache.insert(record);
                if strong_match_found {
                    break;
                }
            }
            Err(MediaFingerprintError::Cancelled { .. }) => {
                return Err("Media Matching index rebuild was canceled.".to_owned());
            }
            Err(_) => {
                skipped += 1;
                if path_needs_fingerprint {
                    fresh_work_done += 1;
                }
            }
        }
    }

    let (current_decision, nearest_match, last_evidence) = summarize_current_media_match(
        request.root,
        request.current_player_path,
        &next_cache,
        request.settings,
        request.extraction_settings,
    );
    let cache_status = media_match_cache_status(request.root);
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
        "Media Matching indexed {scope} ({} reused, {} fingerprinted, {} skipped; {}).",
        reused,
        fingerprinted,
        skipped,
        instrumentation.summary()
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
        nearest_match,
        last_evidence,
    })
}

fn initial_media_match_rebuild_cache(
    existing_cache: &MediaMatchCache,
    prefiltered: bool,
) -> MediaMatchCache {
    if prefiltered {
        existing_cache.clone()
    } else {
        MediaMatchCache::default()
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
    match media_match_sqlite_all_settings_counts(root) {
        Ok((inventory, fast, full)) if inventory > 0 || fast > 0 || full > 0 => {
            let storage = media_match_sqlite_storage_status(root).unwrap_or_default();
            let active = media_match_sqlite_active_settings_counts(root)
                .map(|(active_fast, active_full)| {
                    format!("active settings fast/full={active_fast}/{active_full}")
                })
                .unwrap_or_default();
            let details = [active, storage]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            let details = if details.is_empty() {
                String::new()
            } else {
                format!("; {}", details.join("; "))
            };
            format!(
                "inventory: {inventory}, fast(all settings): {fast}, full(all settings): {full}{details}"
            )
        }
        Ok(_) => "empty".to_owned(),
        Err(error) => format!("unreadable cache: {error}"),
    }
}

fn media_match_sqlite_storage_status(root: &Path) -> Result<String, String> {
    let connection = open_media_match_sqlite_index(root)?;
    let db_bytes = fs::metadata(managed_media_match_index_path(root))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let (summary_bytes, fingerprint_rows, audio_anchors, video_anchors) = connection
        .query_row(
            "SELECT
                COALESCE(SUM(COALESCE(LENGTH(audio_summary), 0) + COALESCE(LENGTH(video_summary), 0)), 0),
                COUNT(*),
                COALESCE(SUM(audio_anchor_count), 0),
                COALESCE(SUM(video_anchor_count), 0)
             FROM fingerprints
             WHERE version = ?1",
            [i64::from(MEDIA_MATCH_ANCHOR_VERSION)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(|error| format!("failed reading media-match storage summary: {error}"))?;
    if fingerprint_rows <= 0 {
        return Ok(String::new());
    }
    let average = summary_bytes as f64 / fingerprint_rows as f64;
    Ok(format!(
        "db={db_bytes} bytes, v2 summaries: {summary_bytes} bytes ({average:.0}/fingerprint), anchors audio/video={audio_anchors}/{video_anchors}"
    ))
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

fn inventory_media_match_candidates(
    root: &Path,
    search_roots: &[PathBuf],
    candidates: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), String> {
    let connection = open_media_match_sqlite_index(root)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match inventory transaction: {error}"))?;
    for path in candidates {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("Media Matching inventory scan was canceled.".to_owned());
        }
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let normalized_path = normalize_media_path(path);
        let size_bytes = metadata.len();
        if let Some((file_id, old_mtime, old_size)) = transaction
            .query_row(
                "SELECT file_id, modified_unix_millis, size_bytes
                 FROM media_files
                 WHERE normalized_path = ?1",
                [normalized_path.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("failed reading media-match inventory row: {error}"))?
            && (old_mtime != modified_unix_millis as i64 || old_size != size_bytes as i64)
        {
            delete_media_match_fingerprints_and_anchors(&transaction, file_id)?;
        }
        transaction
            .execute(
                "INSERT INTO media_files (
                    normalized_path,
                    modified_unix_millis,
                    size_bytes,
                    duration_ms,
                    container_format,
                    video_codec,
                    audio_codec,
                    partial_content_hash,
                    updated_unix_millis
                ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, NULL, ?4)
                ON CONFLICT(normalized_path) DO UPDATE SET
                    modified_unix_millis = excluded.modified_unix_millis,
                    size_bytes = excluded.size_bytes,
                    updated_unix_millis = excluded.updated_unix_millis",
                params![
                    normalized_path,
                    modified_unix_millis as i64,
                    size_bytes as i64,
                    current_unix_millis() as i64,
                ],
            )
            .map_err(|error| format!("failed writing media-match inventory row: {error}"))?;
    }
    prune_missing_media_match_inventory_rows(&transaction, search_roots, candidates)?;
    transaction
        .commit()
        .map_err(|error| format!("failed committing media-match inventory transaction: {error}"))?;
    Ok(())
}

fn prune_missing_media_match_inventory_rows(
    connection: &Connection,
    search_roots: &[PathBuf],
    candidates: &[PathBuf],
) -> Result<(), String> {
    let normalized_roots = search_roots
        .iter()
        .map(normalize_media_path)
        .collect::<Vec<_>>();
    if normalized_roots.is_empty() {
        return Ok(());
    }
    let seen_paths = candidates
        .iter()
        .map(normalize_media_path)
        .collect::<BTreeSet<_>>();
    let mut statement = connection
        .prepare("SELECT file_id, normalized_path FROM media_files")
        .map_err(|error| format!("failed preparing media-match stale inventory query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("failed querying media-match stale inventory rows: {error}"))?;
    let mut stale_file_ids = Vec::new();
    for row in rows.flatten() {
        let (file_id, normalized_path) = row;
        let under_scanned_root = normalized_roots
            .iter()
            .any(|root| media_match_path_is_under_root(&normalized_path, root));
        if under_scanned_root && !seen_paths.contains(&normalized_path) {
            stale_file_ids.push(file_id);
        }
    }
    for file_id in stale_file_ids {
        delete_media_match_file_and_fingerprints(connection, file_id)?;
    }
    Ok(())
}

fn media_match_path_is_under_root(normalized_path: &str, normalized_root: &str) -> bool {
    normalized_path == normalized_root
        || normalized_path
            .strip_prefix(normalized_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
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
        // Missing fingerprints cannot be anchor-shortlisted yet; for large roots this filename
        // pass is intentionally only a fast bootstrap until background warmup adds anchors.
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
    existing_cache: &MediaMatchCache,
    path: &Path,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
) -> Result<
    (
        MediaFingerprintRecord,
        bool,
        Option<sorotte_media_match::MediaFingerprintExtractionReport>,
    ),
    MediaFingerprintError,
> {
    let metadata = fs::metadata(path).map_err(|error| MediaFingerprintError::FileMetadata {
        path: path.display().to_string(),
        error: error.to_string(),
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
        return Ok((record.clone(), true, None));
    }
    fingerprint_media_file_cancellable_with_report(
        path,
        tools,
        extraction_settings,
        cancel_flag.unwrap_or(&AtomicBool::new(false)),
    )
    .map(|fingerprint| (fingerprint.record, false, Some(fingerprint.report)))
}

fn media_match_cache_has_valid_record(
    existing_cache: &MediaMatchCache,
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

fn media_match_anchor_candidate_paths(
    root: &Path,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<Vec<String>, String> {
    if !managed_media_match_index_path(root).exists() {
        return Ok(Vec::new());
    }
    let connection = open_media_match_sqlite_index(root)?;
    let Some(current_file_id) = connection
        .query_row(
            "SELECT file_id FROM media_files WHERE normalized_path = ?1",
            [normalized_current_path],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("failed reading current media-match file id: {error}"))?
    else {
        return Ok(Vec::new());
    };
    let profile = extraction_settings.profile.label();
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let mut scores = BTreeMap::<i64, i64>::new();
    collect_audio_anchor_candidate_scores(
        &connection,
        current_file_id,
        profile,
        &settings_hash,
        &mut scores,
    )?;
    collect_video_anchor_candidate_scores(
        &connection,
        current_file_id,
        profile,
        &settings_hash,
        &mut scores,
    )?;
    let mut scored = scores
        .into_iter()
        .filter(|(file_id, _)| *file_id != current_file_id)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let mut paths = Vec::new();
    for (file_id, _) in scored.into_iter().take(MEDIA_MATCH_PREFILTER_LIMIT) {
        if let Ok(path) = connection.query_row(
            "SELECT normalized_path FROM media_files WHERE file_id = ?1",
            [file_id],
            |row| row.get::<_, String>(0),
        ) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn collect_audio_anchor_candidate_scores(
    connection: &Connection,
    current_file_id: i64,
    profile: &str,
    settings_hash: &[u8],
    scores: &mut BTreeMap<i64, i64>,
) -> Result<(), String> {
    let mut bucket_statement = connection
        .prepare(
            "SELECT DISTINCT anchors.bucket
             FROM audio_anchors anchors
             JOIN fingerprints fingerprints
                ON fingerprints.file_id = anchors.file_id
               AND fingerprints.version = anchors.version
               AND fingerprints.profile = anchors.profile
             WHERE anchors.version = ?1
                AND anchors.profile = ?2
                AND anchors.file_id = ?3
                AND fingerprints.settings_hash = ?4",
        )
        .map_err(|error| format!("failed preparing media-match anchor bucket query: {error}"))?;
    let buckets = bucket_statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                profile,
                current_file_id,
                settings_hash,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed querying media-match anchor buckets: {error}"))?
        .flatten()
        .collect::<BTreeSet<_>>();
    let mut hit_statement = connection
        .prepare(
            "SELECT anchors.file_id, COUNT(*)
             FROM audio_anchors anchors
             JOIN fingerprints fingerprints
                ON fingerprints.file_id = anchors.file_id
               AND fingerprints.version = anchors.version
               AND fingerprints.profile = anchors.profile
             WHERE anchors.version = ?1
                AND anchors.profile = ?2
                AND anchors.bucket = ?3
                AND anchors.file_id != ?4
                AND fingerprints.settings_hash = ?5
             GROUP BY anchors.file_id",
        )
        .map_err(|error| format!("failed preparing media-match anchor hit query: {error}"))?;
    for bucket in buckets {
        let hits = hit_statement
            .query_map(
                params![
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    profile,
                    bucket,
                    current_file_id,
                    settings_hash,
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|error| format!("failed querying media-match anchor hits: {error}"))?;
        for hit in hits.flatten() {
            *scores.entry(hit.0).or_default() += hit.1;
        }
    }
    Ok(())
}

fn collect_video_anchor_candidate_scores(
    connection: &Connection,
    current_file_id: i64,
    profile: &str,
    settings_hash: &[u8],
    scores: &mut BTreeMap<i64, i64>,
) -> Result<(), String> {
    let mut query_statement = connection
        .prepare(
            "SELECT anchors.bucket, anchors.t_ms, anchors.hash64
             FROM video_anchors anchors
             JOIN fingerprints fingerprints
                ON fingerprints.file_id = anchors.file_id
               AND fingerprints.version = anchors.version
               AND fingerprints.profile = anchors.profile
             WHERE anchors.version = ?1
                AND anchors.profile = ?2
                AND anchors.file_id = ?3
                AND fingerprints.settings_hash = ?4",
        )
        .map_err(|error| format!("failed preparing media-match video anchor query: {error}"))?;
    let query_anchors = query_statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                profile,
                current_file_id,
                settings_hash,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            },
        )
        .map_err(|error| format!("failed querying media-match video anchors: {error}"))?
        .flatten()
        .collect::<Vec<_>>();
    let mut hit_statement = connection
        .prepare(
            "SELECT anchors.file_id, anchors.t_ms, anchors.hash64, anchors.weight
             FROM video_anchors anchors
             JOIN fingerprints fingerprints
                ON fingerprints.file_id = anchors.file_id
               AND fingerprints.version = anchors.version
               AND fingerprints.profile = anchors.profile
             WHERE anchors.version = ?1
                AND anchors.profile = ?2
                AND anchors.bucket = ?3
                AND anchors.file_id != ?4
                AND fingerprints.settings_hash = ?5",
        )
        .map_err(|error| format!("failed preparing media-match video anchor hit query: {error}"))?;
    let mut seen_hits = BTreeSet::<(i64, i64, i64, u64, u64)>::new();
    let mut lsh_candidate_ids = BTreeSet::<i64>::new();
    for (bucket, query_t_ms, query_hash64) in &query_anchors {
        let hits = hit_statement
            .query_map(
                params![
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    profile,
                    bucket,
                    current_file_id,
                    settings_hash,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("failed querying media-match video anchor hits: {error}"))?;
        for hit in hits.flatten() {
            if !video_anchor_hashes_match(*query_hash64, hit.2) {
                continue;
            }
            if !seen_hits.insert((hit.0, *query_t_ms, hit.1, *query_hash64, hit.2)) {
                continue;
            }
            lsh_candidate_ids.insert(hit.0);
            *scores.entry(hit.0).or_default() += hit.3.max(1);
        }
    }
    if lsh_candidate_ids.len() < MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MIN_CANDIDATES {
        for (query_t_ms, query_hash64) in
            bounded_video_hamming_fallback_query_anchors(&query_anchors)
        {
            collect_video_anchor_hamming_fallback_scores(
                connection,
                current_file_id,
                profile,
                settings_hash,
                query_t_ms,
                query_hash64,
                scores,
                &mut seen_hits,
            )?;
        }
    }
    Ok(())
}

fn bounded_video_hamming_fallback_query_anchors(
    query_anchors: &[(i64, i64, u64)],
) -> Vec<(i64, u64)> {
    let unique = query_anchors
        .iter()
        .map(|(_, t_ms, hash64)| (*t_ms, *hash64))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if unique.len() <= MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_QUERY_ANCHORS {
        return unique;
    }
    let stride = unique.len() as f64 / MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_QUERY_ANCHORS as f64;
    (0..MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_QUERY_ANCHORS)
        .map(|index| unique[(index as f64 * stride).floor() as usize])
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn collect_video_anchor_hamming_fallback_scores(
    connection: &Connection,
    current_file_id: i64,
    profile: &str,
    settings_hash: &[u8],
    query_t_ms: i64,
    query_hash64: u64,
    scores: &mut BTreeMap<i64, i64>,
    seen_hits: &mut BTreeSet<(i64, i64, i64, u64, u64)>,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(
            "SELECT anchors.file_id, anchors.t_ms, anchors.hash64, anchors.weight
             FROM video_anchors anchors
             JOIN fingerprints fingerprints
                ON fingerprints.file_id = anchors.file_id
               AND fingerprints.version = anchors.version
               AND fingerprints.profile = anchors.profile
             WHERE anchors.version = ?1
                AND anchors.profile = ?2
                AND anchors.file_id != ?3
                AND fingerprints.settings_hash = ?4
             ORDER BY anchors.file_id, anchors.t_ms
             LIMIT ?5",
        )
        .map_err(|error| {
            format!("failed preparing media-match video Hamming fallback query: {error}")
        })?;
    let hits = statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                profile,
                current_file_id,
                settings_hash,
                MEDIA_MATCH_VIDEO_HAMMING_FALLBACK_MAX_ROWS_PER_QUERY,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(|error| format!("failed querying media-match video Hamming fallback: {error}"))?;
    for hit in hits.flatten() {
        if !video_anchor_hashes_match(query_hash64, hit.2) {
            continue;
        }
        if !seen_hits.insert((hit.0, query_t_ms, hit.1, query_hash64, hit.2)) {
            continue;
        }
        *scores.entry(hit.0).or_default() += hit.3.max(1);
    }
    Ok(())
}

fn summarize_current_media_match(
    root: &Path,
    current_player_path: Option<&str>,
    cache: &MediaMatchCache,
    settings: &MediaMatchSettings,
    extraction_settings: &MediaExtractionSettings,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(current_player_path) = current_player_path else {
        return (
            Some("unknown: no resolved current local file".to_owned()),
            None,
            None,
        );
    };
    let normalized_current_path = normalize_media_path(current_player_path);
    let Some(query) = cache.records.get(&normalized_current_path) else {
        return (
            Some("unknown: current player file is not indexed".to_owned()),
            None,
            None,
        );
    };
    let anchor_candidates =
        media_match_anchor_candidate_paths(root, &normalized_current_path, extraction_settings)
            .unwrap_or_default();
    let anchor_candidate_set = anchor_candidates.iter().cloned().collect::<BTreeSet<_>>();
    let use_anchor_candidates = !anchor_candidate_set.is_empty();
    let ranked = rank_media_match_candidates(
        query,
        cache.records.values().filter(|record| {
            record.identity.normalized_path != normalized_current_path
                && (!use_anchor_candidates
                    || anchor_candidate_set.contains(&record.identity.normalized_path))
        }),
        settings,
    );
    let Some(best) = ranked.into_iter().next() else {
        return (
            Some("exact: current local file is indexed".to_owned()),
            None,
            Some("current file is indexed exactly; no alternate indexed candidates".to_owned()),
        );
    };
    if matches!(
        best.decision.tier,
        MediaMatchTier::Reject | MediaMatchTier::Unknown
    ) {
        return (
            Some("exact: current local file is indexed".to_owned()),
            Some(format!(
                "No alternate indexed match; nearest other: {}",
                format_media_match_nearest_candidate(&best)
            )),
            Some(format!(
                "current file is indexed exactly | nearest_other {}",
                format_media_match_evidence_summary(&best.decision)
            )),
        );
    }
    let tier = media_match_tier_label(best.decision.tier);
    (
        Some(format!("{tier}: {}", best.decision.explanation)),
        Some(format_media_match_nearest_candidate(&best)),
        Some(format_media_match_evidence_summary(&best.decision)),
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

fn format_media_match_nearest_candidate(candidate: &MediaMatchCandidateDecision) -> String {
    let tier = media_match_tier_label(candidate.decision.tier);
    format!(
        "{} ({tier}: {})",
        candidate.candidate_path, candidate.decision.explanation
    )
}

fn format_media_match_evidence_summary(decision: &MediaMatchDecision) -> String {
    let mut parts = vec![format!(
        "tier={} reason={}",
        media_match_tier_label(decision.tier),
        decision.explanation
    )];
    if let Some(alignment) = decision.evidence.alignment.as_ref() {
        parts.push(format!(
            "alignment offset={:.1}s scale={}ppm drift={:.4} span={:.1}s pairs={} audio={} video={} margin={:.2}",
            alignment.offset_seconds,
            alignment.scale_ppm,
            alignment.drift_ratio,
            alignment.aligned_span_seconds,
            alignment.aligned_pairs,
            alignment.aligned_audio_anchors,
            alignment.aligned_video_anchors,
            alignment.second_best_offset_margin
        ));
    }
    if let Some(audio) = decision.evidence.audio.as_ref() {
        parts.push(format!(
            "audio similarity={:.2} shared={:.2} duration_delta={}",
            audio.similarity,
            audio.shared_token_ratio,
            format_optional_seconds(audio.duration_delta_seconds)
        ));
    }
    if let Some(video) = decision.evidence.video.as_ref() {
        parts.push(format!(
            "video pairs={} coverage={:.2}/{:.2} offset={:.1}s drift={:.4} mean_hamming={:.1}",
            video.aligned_pairs,
            video.query_coverage,
            video.candidate_coverage,
            video.best_offset_seconds,
            video.drift_ratio,
            video.mean_hamming_distance
        ));
    }
    parts.push(format!(
        "metadata duration_delta={} duration_within_tolerance={}",
        format_optional_seconds(decision.evidence.metadata.duration_delta_seconds),
        format_optional_bool(decision.evidence.metadata.duration_within_tolerance)
    ));
    if !decision.evidence.notes.is_empty() {
        parts.push(format!("notes={}", decision.evidence.notes.join("; ")));
    }
    parts.join(" | ")
}

fn format_optional_seconds(value: Option<f64>) -> String {
    value
        .map(|seconds| format!("{seconds:.1}s"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "n/a",
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
    drop_legacy_media_match_tables(&connection)?;
    initialize_media_match_sqlite_index(&connection)?;
    Ok(connection)
}

fn drop_legacy_media_match_tables(connection: &Connection) -> Result<(), String> {
    let table_exists = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'fingerprints'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed checking media-match fingerprints table: {error}"))?
        > 0;
    if table_exists {
        let mut statement = connection
            .prepare("PRAGMA table_info(fingerprints)")
            .map_err(|error| {
                format!("failed reading media-match fingerprints table info: {error}")
            })?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| format!("failed reading media-match fingerprints columns: {error}"))?
            .flatten()
            .collect::<BTreeSet<_>>();
        if columns.contains("record_json") {
            connection
                .execute("DROP TABLE fingerprints", [])
                .map_err(|error| format!("failed dropping legacy media-match table: {error}"))?;
        }
    }
    connection
        .execute("DROP TABLE IF EXISTS fingerprints_v1", [])
        .map_err(|error| format!("failed dropping legacy media-match v1 table: {error}"))?;
    Ok(())
}

fn initialize_media_match_sqlite_index(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS media_files (
                file_id INTEGER PRIMARY KEY,
                normalized_path TEXT NOT NULL UNIQUE,
                modified_unix_millis INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                duration_ms INTEGER,
                container_format TEXT,
                video_codec TEXT,
                audio_codec TEXT,
                partial_content_hash BLOB,
                updated_unix_millis INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS fingerprints (
                file_id INTEGER NOT NULL,
                version INTEGER NOT NULL,
                profile TEXT NOT NULL,
                status TEXT NOT NULL,
                settings_hash BLOB NOT NULL,
                duration_ms INTEGER,
                audio_summary BLOB,
                video_summary BLOB,
                audio_anchor_count INTEGER NOT NULL DEFAULT 0,
                video_anchor_count INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                updated_unix_millis INTEGER NOT NULL,
                PRIMARY KEY (file_id, version, profile),
                FOREIGN KEY (file_id) REFERENCES media_files(file_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_media_match_fingerprints_profile
                ON fingerprints(version, profile);
            CREATE TABLE IF NOT EXISTS audio_anchors (
                version INTEGER NOT NULL,
                profile TEXT NOT NULL,
                bucket INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                t_ms INTEGER NOT NULL,
                weight INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (version, profile, bucket, file_id, t_ms)
            );
            CREATE INDEX IF NOT EXISTS idx_audio_anchor_lookup
                ON audio_anchors(version, profile, bucket);
            CREATE TABLE IF NOT EXISTS video_anchors (
                version INTEGER NOT NULL,
                profile TEXT NOT NULL,
                bucket INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                t_ms INTEGER NOT NULL,
                hash64 INTEGER NOT NULL,
                weight INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (version, profile, bucket, file_id, t_ms)
            );
            CREATE INDEX IF NOT EXISTS idx_video_anchor_lookup
                ON video_anchors(version, profile, bucket);
            ",
        )
        .map_err(|error| format!("failed initializing media-match SQLite index: {error}"))?;
    let user_version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| format!("failed reading media-match SQLite schema version: {error}"))?;
    if user_version == MEDIA_MATCH_SQLITE_SCHEMA_VERSION {
        return Ok(());
    }
    connection
        .pragma_update(None, "user_version", MEDIA_MATCH_SQLITE_SCHEMA_VERSION)
        .map_err(|error| format!("failed setting media-match SQLite schema version: {error}"))?;
    Ok(())
}

fn media_match_profile_label(settings: &MediaExtractionSettings) -> &'static str {
    settings.profile.label()
}

fn delete_media_match_fingerprints_and_anchors(
    connection: &Connection,
    file_id: i64,
) -> Result<(), String> {
    connection
        .execute("DELETE FROM audio_anchors WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match audio anchors: {error}"))?;
    connection
        .execute("DELETE FROM video_anchors WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match video anchors: {error}"))?;
    connection
        .execute("DELETE FROM fingerprints WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match fingerprints: {error}"))?;
    Ok(())
}

fn delete_media_match_file_and_fingerprints(
    connection: &Connection,
    file_id: i64,
) -> Result<(), String> {
    delete_media_match_fingerprints_and_anchors(connection, file_id)?;
    connection
        .execute("DELETE FROM media_files WHERE file_id = ?1", [file_id])
        .map_err(|error| format!("failed deleting stale media-match file row: {error}"))?;
    Ok(())
}

pub(super) fn load_media_match_cache_for_settings(
    root: &Path,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchCache> {
    let connection = open_media_match_sqlite_index(root).ok()?;
    load_media_match_cache_for_settings_from_sqlite(&connection, extraction_settings)
        .filter(|cache| !cache.records.is_empty())
}

fn load_media_match_cache_for_settings_from_sqlite(
    connection: &Connection,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchCache> {
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let mut statement = connection
        .prepare(
            "SELECT
                media_files.normalized_path,
                media_files.modified_unix_millis,
                media_files.size_bytes,
                media_files.duration_ms,
                media_files.container_format,
                fingerprints.duration_ms,
                fingerprints.audio_summary,
                fingerprints.video_summary,
                fingerprints.error
             FROM fingerprints
             JOIN media_files ON media_files.file_id = fingerprints.file_id
             WHERE fingerprints.version = ?1
                AND fingerprints.profile = ?2
                AND fingerprints.settings_hash = ?3",
        )
        .ok()?;
    let rows = statement
        .query_map(
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                media_match_profile_label(extraction_settings),
                settings_hash,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .ok()?;
    let mut cache = MediaMatchCache::default();
    for row in rows.flatten() {
        let (
            normalized_path,
            modified_unix_millis,
            size_bytes,
            media_duration_ms,
            container_format,
            fingerprint_duration_ms,
            audio_summary,
            video_summary,
            error,
        ) = row;
        let record = media_match_record_from_cached_summary(
            normalized_path.clone(),
            modified_unix_millis,
            size_bytes,
            media_duration_ms,
            container_format,
            fingerprint_duration_ms,
            audio_summary,
            video_summary,
            error,
            extraction_settings,
        );
        cache.insert(record);
    }
    Some(cache)
}

#[allow(clippy::too_many_arguments)]
fn media_match_record_from_cached_summary(
    normalized_path: String,
    modified_unix_millis: i64,
    size_bytes: i64,
    media_duration_ms: Option<i64>,
    container_format: Option<String>,
    fingerprint_duration_ms: Option<i64>,
    audio_summary: Option<Vec<u8>>,
    video_summary: Option<Vec<u8>>,
    error: Option<String>,
    extraction_settings: &MediaExtractionSettings,
) -> MediaFingerprintRecord {
    let audio_anchors = audio_summary
        .as_deref()
        .map(decode_audio_anchor_summary)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default();
    let video_anchors = video_summary
        .as_deref()
        .map(decode_video_anchor_summary)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default();
    let duration_ms = fingerprint_duration_ms.or(media_duration_ms);
    let duration_seconds = duration_ms.map(|value| value as f64 / 1000.0);
    let video = (!video_anchors.is_empty()).then(|| sorotte_media_match::VideoFingerprint {
        duration_seconds: duration_ms.map(|value| (value / 1000).min(i64::from(u32::MAX)) as u32),
        frames: video_anchors
            .iter()
            .map(|anchor| {
                sorotte_media_match::FrameFingerprint::new(
                    anchor.t_ms as f64 / 1000.0,
                    anchor.hash64,
                )
            })
            .collect(),
    });
    MediaFingerprintRecord {
        identity: sorotte_media_match::MediaFileIdentity {
            normalized_path: normalized_path.clone(),
            modified_unix_millis: modified_unix_millis.max(0) as u64,
            size_bytes: size_bytes.max(0) as u64,
        },
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: extraction_settings.clone(),
        duration_seconds,
        container_fingerprint: container_format.unwrap_or_else(|| {
            sorotte_media_match::container_fingerprint_from_metadata(
                &normalized_path,
                modified_unix_millis.max(0) as u64,
                size_bytes.max(0) as u64,
                duration_seconds,
            )
        }),
        audio: None,
        video,
        audio_anchors,
        video_anchors,
        audio_error: error.clone(),
        video_error: error,
    }
}

pub(super) fn media_match_wire_value_for_path(
    root: &Path,
    current_player_path: &str,
) -> Option<serde_json::Value> {
    let fast_record = media_match_record_for_path(
        root,
        current_player_path,
        &MediaExtractionSettings::fast_anchor_v2(),
    )?;
    let mut records = vec![fast_record.clone()];
    if let Some(full_record) = media_match_record_for_path(
        root,
        current_player_path,
        &MediaExtractionSettings::full_anchor_v2(),
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
    let connection = open_media_match_sqlite_index(root).ok()?;
    load_media_match_record_for_path_from_sqlite(
        &connection,
        &normalized_path,
        extraction_settings,
        modified_unix_millis,
        size_bytes,
    )
}

fn load_media_match_record_for_path_from_sqlite(
    connection: &Connection,
    normalized_path: &str,
    extraction_settings: &MediaExtractionSettings,
    modified_unix_millis: u64,
    size_bytes: u64,
) -> Option<MediaFingerprintRecord> {
    let settings_hash = media_extraction_settings_hash(extraction_settings).to_vec();
    let row = connection
        .query_row(
            "SELECT
                media_files.normalized_path,
                media_files.modified_unix_millis,
                media_files.size_bytes,
                media_files.duration_ms,
                media_files.container_format,
                fingerprints.duration_ms,
                fingerprints.audio_summary,
                fingerprints.video_summary,
                fingerprints.error
             FROM fingerprints
             JOIN media_files ON media_files.file_id = fingerprints.file_id
             WHERE fingerprints.version = ?1
                AND fingerprints.profile = ?2
                AND fingerprints.settings_hash = ?3
                AND media_files.normalized_path = ?4",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                media_match_profile_label(extraction_settings),
                settings_hash,
                normalized_path,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()
        .ok()??;
    let record = media_match_record_from_cached_summary(
        row.0,
        row.1,
        row.2,
        row.3,
        row.4,
        row.5,
        row.6,
        row.7,
        row.8,
        extraction_settings,
    );
    record
        .valid_for(
            normalized_path,
            modified_unix_millis,
            size_bytes,
            MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings,
        )
        .then_some(record)
}

#[cfg(test)]
fn save_media_match_cache(root: &Path, cache: &MediaMatchCache) -> Result<(), String> {
    let connection = open_media_match_sqlite_index(root)?;
    save_media_match_cache_to_sqlite(&connection, cache)
}

#[cfg(test)]
fn save_media_match_cache_to_sqlite(
    connection: &Connection,
    cache: &MediaMatchCache,
) -> Result<(), String> {
    for record in cache.records.values() {
        save_media_match_record_to_sqlite(connection, record)?;
    }
    Ok(())
}

fn save_media_match_record_to_sqlite(
    connection: &Connection,
    record: &MediaFingerprintRecord,
) -> Result<(), String> {
    save_media_match_record_to_sqlite_with_error(connection, record, None)
}

fn save_media_match_record_to_sqlite_with_error(
    connection: &Connection,
    record: &MediaFingerprintRecord,
    error: Option<&str>,
) -> Result<(), String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("failed starting media-match save transaction: {error}"))?;
    let now = current_unix_millis() as i64;
    let duration_ms = record
        .duration_seconds
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| (value * 1000.0).round().min(f64::from(u32::MAX)) as i64);
    if let Some((file_id, old_mtime, old_size)) = transaction
        .query_row(
            "SELECT file_id, modified_unix_millis, size_bytes
             FROM media_files
             WHERE normalized_path = ?1",
            [record.identity.normalized_path.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("failed reading media-match media file row: {error}"))?
        && (old_mtime != record.identity.modified_unix_millis as i64
            || old_size != record.identity.size_bytes as i64)
    {
        delete_media_match_fingerprints_and_anchors(&transaction, file_id)?;
    }
    transaction
        .execute(
            "INSERT INTO media_files (
                normalized_path,
                modified_unix_millis,
                size_bytes,
                duration_ms,
                container_format,
                video_codec,
                audio_codec,
                partial_content_hash,
                updated_unix_millis
            ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, ?6)
            ON CONFLICT(normalized_path) DO UPDATE SET
                modified_unix_millis = excluded.modified_unix_millis,
                size_bytes = excluded.size_bytes,
                duration_ms = excluded.duration_ms,
                container_format = excluded.container_format,
                updated_unix_millis = excluded.updated_unix_millis",
            params![
                record.identity.normalized_path,
                record.identity.modified_unix_millis as i64,
                record.identity.size_bytes as i64,
                duration_ms,
                record.container_fingerprint,
                now,
            ],
        )
        .map_err(|error| format!("failed writing media-match media file row: {error}"))?;
    let file_id = transaction
        .query_row(
            "SELECT file_id FROM media_files WHERE normalized_path = ?1",
            [record.identity.normalized_path.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("failed reading media-match file id: {error}"))?;
    let summary = media_fingerprint_summary_from_record(record);
    let combined_error = error.map(str::to_owned).or_else(|| {
        let mut errors = Vec::new();
        if let Some(audio_error) = &record.audio_error {
            errors.push(format!("audio: {audio_error}"));
        }
        if let Some(video_error) = &record.video_error {
            errors.push(format!("video: {video_error}"));
        }
        (!errors.is_empty()).then(|| errors.join("; "))
    });
    let status = if error.is_some() {
        "error"
    } else if combined_error.is_some() {
        "partial"
    } else if summary.audio_anchor_count == 0 && summary.video_anchor_count == 0 {
        "empty"
    } else {
        "complete"
    };
    transaction
        .execute(
            "DELETE FROM audio_anchors WHERE version = ?1 AND profile = ?2 AND file_id = ?3",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                record.extraction_settings.profile.label(),
                file_id,
            ],
        )
        .map_err(|error| format!("failed clearing media-match audio anchors: {error}"))?;
    transaction
        .execute(
            "DELETE FROM video_anchors WHERE version = ?1 AND profile = ?2 AND file_id = ?3",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                record.extraction_settings.profile.label(),
                file_id,
            ],
        )
        .map_err(|error| format!("failed clearing media-match video anchors: {error}"))?;
    let audio_anchors = audio_anchors_from_record(record);
    for anchor in &audio_anchors {
        insert_audio_anchor(
            &transaction,
            file_id,
            record.extraction_settings.profile.label(),
            anchor,
        )?;
    }
    let video_anchors = video_anchors_from_record(record);
    for anchor in &video_anchors {
        insert_video_anchor(
            &transaction,
            file_id,
            record.extraction_settings.profile.label(),
            anchor,
        )?;
    }
    transaction
        .execute(
            "INSERT OR REPLACE INTO fingerprints (
                file_id,
                version,
                profile,
                status,
                settings_hash,
                duration_ms,
                audio_summary,
                video_summary,
                audio_anchor_count,
                video_anchor_count,
                error,
                updated_unix_millis
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                file_id,
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                record.extraction_settings.profile.label(),
                status,
                media_extraction_settings_hash(&record.extraction_settings).to_vec(),
                duration_ms,
                summary.audio_summary,
                summary.video_summary,
                summary.audio_anchor_count as i64,
                summary.video_anchor_count as i64,
                combined_error,
                now,
            ],
        )
        .map_err(|error| format!("failed checkpointing media-match v2 fingerprint row: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed committing media-match save transaction: {error}"))
}

fn insert_audio_anchor(
    connection: &Connection,
    file_id: i64,
    profile: &str,
    anchor: &AudioAnchor,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR REPLACE INTO audio_anchors (
                version, profile, bucket, file_id, t_ms, weight
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                profile,
                i64::from(anchor.bucket),
                file_id,
                i64::from(anchor.t_ms),
                i64::from(anchor.weight),
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("failed writing media-match audio anchor: {error}"))
}

fn insert_video_anchor(
    connection: &Connection,
    file_id: i64,
    profile: &str,
    anchor: &VideoAnchor,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR REPLACE INTO video_anchors (
                version, profile, bucket, file_id, t_ms, hash64, weight
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                profile,
                i64::from(anchor.bucket),
                file_id,
                i64::from(anchor.t_ms),
                anchor.hash64 as i64,
                i64::from(anchor.weight),
            ],
        )
        .map(|_| ())
        .map_err(|error| format!("failed writing media-match video anchor: {error}"))
}

fn media_match_sqlite_all_settings_counts(root: &Path) -> Result<(usize, usize, usize), String> {
    if !managed_media_match_index_path(root).exists() {
        return Ok((0, 0, 0));
    }
    let connection = open_media_match_sqlite_index(root)?;
    let inventory = connection
        .query_row("SELECT COUNT(*) FROM media_files", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| format!("failed reading media-match inventory count: {error}"))?
        .max(0) as usize;
    let count_for_profile = |profile: &str| -> Result<usize, String> {
        Ok(connection
            .query_row(
                "SELECT COUNT(*) FROM fingerprints WHERE version = ?1 AND profile = ?2",
                params![i64::from(MEDIA_MATCH_ANCHOR_VERSION), profile],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("failed reading media-match profile count: {error}"))?
            .max(0) as usize)
    };
    Ok((
        inventory,
        count_for_profile(MediaFingerprintProfile::FastAnchorV2.label())?,
        count_for_profile(MediaFingerprintProfile::FullAnchorV2.label())?,
    ))
}

fn media_match_sqlite_active_settings_counts(root: &Path) -> Result<(usize, usize), String> {
    if !managed_media_match_index_path(root).exists() {
        return Ok((0, 0));
    }
    let connection = open_media_match_sqlite_index(root)?;
    let count_for_settings = |settings: &MediaExtractionSettings| -> Result<usize, String> {
        Ok(connection
            .query_row(
                "SELECT COUNT(*)
                 FROM fingerprints
                 WHERE version = ?1
                   AND profile = ?2
                   AND settings_hash = ?3",
                params![
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    settings.profile.label(),
                    media_extraction_settings_hash(settings).to_vec(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("failed reading media-match active-settings count: {error}"))?
            .max(0) as usize)
    };
    Ok((
        count_for_settings(&MediaExtractionSettings::fast_anchor_v2())?,
        count_for_settings(&MediaExtractionSettings::full_anchor_v2())?,
    ))
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
            audio_anchors: Vec::new(),
            video_anchors: Vec::new(),
            audio_error: None,
            video_error: None,
        }
    }

    fn fake_media_match_record_for_file(
        path: &Path,
        extraction_settings: MediaExtractionSettings,
    ) -> MediaFingerprintRecord {
        let metadata = std::fs::metadata(path).expect("test media file should exist");
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(
                path,
                modified_unix_millis,
                metadata.len(),
            ),
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings,
            duration_seconds: Some(1200.0),
            container_fingerprint: format!("container:{}", path.display()),
            audio: None,
            video: None,
            audio_anchors: Vec::new(),
            video_anchors: Vec::new(),
            audio_error: None,
            video_error: None,
        }
    }

    fn enabled_media_match_settings() -> MediaMatchSettings {
        MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        }
    }

    fn media_match_record_updated_unix_millis(root: &Path, record: &MediaFingerprintRecord) -> i64 {
        let connection = open_media_match_sqlite_index(root).expect("SQLite index should open");
        connection
            .query_row(
                "SELECT fingerprints.updated_unix_millis
                 FROM fingerprints
                 JOIN media_files ON media_files.file_id = fingerprints.file_id
                 WHERE media_files.normalized_path = ?1
                   AND fingerprints.version = ?2
                   AND fingerprints.profile = ?3",
                params![
                    record.identity.normalized_path,
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    record.extraction_settings.profile.label(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .expect("record timestamp should be readable")
    }

    #[test]
    fn media_match_clear_persisted_cache_removes_cache_and_tool_metadata() {
        let root = unique_media_match_test_root("clear");
        let metadata_dir = managed_media_match_bin_dir(&root);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir should be created");
        let index_path = managed_media_match_index_path(&root);
        let metadata_path = managed_media_match_metadata_path(&root);
        let mut sqlite_cache = MediaMatchCache::default();
        sqlite_cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_cache(&root, &sqlite_cache).expect("SQLite cache should be written");
        assert!(index_path.exists());
        std::fs::write(&metadata_path, r#"{"version":1}"#).expect("metadata should be written");

        clear_persisted_media_match_cache_at_root(&root).expect("clear should succeed");

        assert!(!index_path.exists());
        assert!(!metadata_path.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_index_backup_restore_reinstates_previous_sqlite_index() {
        let root = unique_media_match_test_root("restore");
        let mut previous_cache = MediaMatchCache::default();
        previous_cache.insert(fake_media_match_record("previous.mkv"));
        save_media_match_cache(&root, &previous_cache).expect("previous cache should be written");

        let backup_existed =
            prepare_media_match_index_rebuild_backup(&root).expect("backup should be prepared");
        assert!(backup_existed);

        remove_sqlite_file_set(&managed_media_match_index_path(&root))
            .expect("primary index should be removed for test");
        let mut partial_cache = MediaMatchCache::default();
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

        let mut partial_cache = MediaMatchCache::default();
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
        let mut full_cache = MediaMatchCache::default();
        full_cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_cache(&root, &full_cache).expect("full cache should be written");

        let mut fast_record = fake_media_match_record("episode.mkv");
        fast_record.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        let mut fast_cache = MediaMatchCache::default();
        fast_cache.insert(fast_record);
        save_media_match_cache(&root, &fast_cache).expect("fast cache should be written");

        assert_eq!(
            media_match_sqlite_all_settings_counts(&root).expect("counts should load"),
            (1, 1, 1)
        );
        assert_eq!(
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::fast_anchor_v2())
                .expect("fast cache should load")
                .records
                .len(),
            1
        );
        assert_eq!(
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::full_anchor_v2())
                .expect("full cache should load")
                .records
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_sqlite_loads_only_matching_settings_hash() {
        let root = unique_media_match_test_root("sqlite-settings-hash");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let media_path = media_dir.join("episode.mkv");
        std::fs::write(&media_path, b"episode").expect("media file should be written");
        let mut altered_settings = MediaExtractionSettings::fast_anchor_v2();
        altered_settings.max_frames += 1;
        let mut record = fake_media_match_record_for_file(&media_path, altered_settings.clone());
        record.audio_anchors = vec![AudioAnchor {
            bucket: 1,
            t_ms: 1_000,
            weight: 1,
        }];
        let mut cache = MediaMatchCache::default();
        cache.insert(record);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        assert!(
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::fast_anchor_v2())
                .is_none(),
            "same profile rows with a different settings hash must not be reused"
        );
        assert!(
            media_match_record_for_path(
                &root,
                media_path.to_str().expect("test path should be UTF-8"),
                &MediaExtractionSettings::fast_anchor_v2(),
            )
            .is_none(),
            "direct single-record lookup must also enforce settings_hash"
        );
        assert!(
            load_media_match_cache_for_settings(&root, &altered_settings).is_some(),
            "the row should still load for the exact extraction settings"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_inventory_invalidates_fingerprints_and_anchors_when_file_changes() {
        let root = unique_media_match_test_root("stale-invalidation");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let query_path = media_dir.join("query.mkv");
        let candidate_path = media_dir.join("candidate.mkv");
        std::fs::write(&query_path, b"query-v1").expect("query media should be written");
        std::fs::write(&candidate_path, b"candidate-v1")
            .expect("candidate media should be written");
        let extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        let mut query = fake_media_match_record_for_file(&query_path, extraction_settings.clone());
        query.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 10_000,
            weight: 1,
        }];
        let mut candidate =
            fake_media_match_record_for_file(&candidate_path, extraction_settings.clone());
        candidate.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 11_000,
            weight: 1,
        }];
        let candidate_normalized_path = candidate.identity.normalized_path.clone();
        let mut cache = MediaMatchCache::default();
        cache.insert(query.clone());
        cache.insert(candidate);
        save_media_match_cache(&root, &cache).expect("cache should be written");
        assert!(
            media_match_anchor_candidate_paths(
                &root,
                &query.identity.normalized_path,
                &extraction_settings,
            )
            .expect("anchor candidates should load")
            .contains(&candidate_normalized_path),
            "fixture should start with a candidate anchor hit"
        );

        std::fs::write(&candidate_path, b"candidate-v2-with-new-size")
            .expect("candidate media should be changed");
        inventory_media_match_candidates(
            &root,
            std::slice::from_ref(&media_dir),
            &[query_path.clone(), candidate_path.clone()],
            None,
        )
        .expect("inventory should update");

        let cache = load_media_match_cache_for_settings(&root, &extraction_settings)
            .expect("query record should still load");
        assert!(
            cache.records.contains_key(&query.identity.normalized_path),
            "unchanged query fingerprint should remain"
        );
        assert!(
            !cache.records.contains_key(&candidate_normalized_path),
            "changed candidate fingerprint must be invalidated"
        );
        assert!(
            media_match_record_for_path(
                &root,
                candidate_path.to_str().expect("test path should be UTF-8"),
                &extraction_settings,
            )
            .is_none(),
            "direct lookup must not reconstruct a stale fingerprint from updated media_files identity"
        );
        assert!(
            !media_match_anchor_candidate_paths(
                &root,
                &query.identity.normalized_path,
                &extraction_settings,
            )
            .expect("anchor candidates should load")
            .contains(&candidate_normalized_path),
            "stale anchors for changed files must not be used for candidate lookup"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_save_invalidates_other_profiles_when_file_identity_changes() {
        let root = unique_media_match_test_root("save-cross-profile-invalidation");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let query_path = media_dir.join("query.mkv");
        let candidate_path = media_dir.join("candidate.mkv");
        std::fs::write(&query_path, b"query-v1").expect("query media should be written");
        std::fs::write(&candidate_path, b"candidate-v1")
            .expect("candidate media should be written");
        let fast_settings = MediaExtractionSettings::fast_anchor_v2();
        let full_settings = MediaExtractionSettings::full_anchor_v2();
        let mut full_query = fake_media_match_record_for_file(&query_path, full_settings.clone());
        full_query.audio_anchors = vec![AudioAnchor {
            bucket: 77,
            t_ms: 10_000,
            weight: 1,
        }];
        let mut stale_full_candidate =
            fake_media_match_record_for_file(&candidate_path, full_settings.clone());
        stale_full_candidate.audio_anchors = vec![AudioAnchor {
            bucket: 77,
            t_ms: 12_000,
            weight: 1,
        }];
        let mut stale_fast_candidate =
            fake_media_match_record_for_file(&candidate_path, fast_settings.clone());
        stale_fast_candidate.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 12_000,
            weight: 1,
        }];
        let candidate_normalized_path = stale_full_candidate.identity.normalized_path.clone();
        let connection = open_media_match_sqlite_index(&root).expect("SQLite index should open");
        save_media_match_record_to_sqlite(&connection, &full_query)
            .expect("full query should be saved");
        save_media_match_record_to_sqlite(&connection, &stale_fast_candidate)
            .expect("stale fast candidate should be saved");
        save_media_match_record_to_sqlite(&connection, &stale_full_candidate)
            .expect("stale full candidate should be saved");
        assert!(
            media_match_anchor_candidate_paths(
                &root,
                &full_query.identity.normalized_path,
                &full_settings,
            )
            .expect("full anchor candidates should load")
            .contains(&candidate_normalized_path),
            "fixture should start with a full-profile candidate hit"
        );

        std::fs::write(&candidate_path, b"candidate-v2-with-new-size")
            .expect("candidate media should be changed");
        let mut fresh_fast_candidate =
            fake_media_match_record_for_file(&candidate_path, fast_settings.clone());
        fresh_fast_candidate.audio_anchors = vec![AudioAnchor {
            bucket: 43,
            t_ms: 12_000,
            weight: 1,
        }];
        save_media_match_record_to_sqlite(&connection, &fresh_fast_candidate)
            .expect("fresh fast candidate should replace stale profiles atomically");

        assert!(
            media_match_record_for_path(
                &root,
                candidate_path.to_str().expect("test path should be UTF-8"),
                &full_settings,
            )
            .is_none(),
            "saving a fresh fast profile for a changed file must invalidate stale full profile rows"
        );
        assert!(
            !media_match_anchor_candidate_paths(
                &root,
                &full_query.identity.normalized_path,
                &full_settings,
            )
            .expect("full anchor candidates should load")
            .contains(&candidate_normalized_path),
            "stale full-profile anchors must not remain candidate lookup evidence"
        );
        let fresh_fast = media_match_record_for_path(
            &root,
            candidate_path.to_str().expect("test path should be UTF-8"),
            &fast_settings,
        )
        .expect("fresh fast profile should load");
        assert_eq!(
            fresh_fast.identity.size_bytes,
            fresh_fast_candidate.identity.size_bytes
        );
        assert!(
            audio_anchors_from_record(&fresh_fast)
                .iter()
                .any(|anchor| anchor.bucket == 43)
        );
        let full_rows = connection
            .query_row(
                "SELECT COUNT(*)
                 FROM fingerprints
                 JOIN media_files ON media_files.file_id = fingerprints.file_id
                 WHERE media_files.normalized_path = ?1
                   AND fingerprints.version = ?2
                   AND fingerprints.profile = ?3",
                params![
                    candidate_normalized_path,
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    full_settings.profile.label(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .expect("full row count should load");
        assert_eq!(full_rows, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_inventory_prunes_deleted_files_under_scanned_roots() {
        let root = unique_media_match_test_root("deleted-inventory-prune");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let kept_path = media_dir.join("kept.mkv");
        let removed_path = media_dir.join("removed.mkv");
        std::fs::write(&kept_path, b"kept").expect("kept media should be written");
        std::fs::write(&removed_path, b"removed").expect("removed media should be written");
        let extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        let mut cache = MediaMatchCache::default();
        let kept = fake_media_match_record_for_file(&kept_path, extraction_settings.clone());
        let removed = fake_media_match_record_for_file(&removed_path, extraction_settings.clone());
        let removed_normalized_path = removed.identity.normalized_path.clone();
        cache.insert(kept.clone());
        cache.insert(removed);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        std::fs::remove_file(&removed_path).expect("removed media should be deleted");
        inventory_media_match_candidates(
            &root,
            std::slice::from_ref(&media_dir),
            std::slice::from_ref(&kept_path),
            None,
        )
        .expect("inventory should prune deleted files");

        let cache = load_media_match_cache_for_settings(&root, &extraction_settings)
            .expect("kept cache should still load");
        assert!(cache.records.contains_key(&kept.identity.normalized_path));
        assert!(
            !cache.records.contains_key(&removed_normalized_path),
            "deleted files under scanned roots should not remain in the media-match cache"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_anchor_candidates_use_video_lsh_and_hamming_verification() {
        let root = unique_media_match_test_root("video-lsh-candidates");
        let query_hash = 0x0123_4567_89ab_cdef;
        let candidate_hash = query_hash ^ (1 << 60);
        let mut query = fake_media_match_record("query.mkv");
        query.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        query.video = Some(sorotte_media_match::VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![sorotte_media_match::FrameFingerprint {
                timestamp_millis: 30_000,
                hash: query_hash,
            }],
        });
        let mut candidate = fake_media_match_record("candidate.mkv");
        candidate.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        candidate.video = Some(sorotte_media_match::VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![sorotte_media_match::FrameFingerprint {
                timestamp_millis: 31_000,
                hash: candidate_hash,
            }],
        });
        let candidate_path = candidate.identity.normalized_path.clone();
        let mut cache = MediaMatchCache::default();
        cache.insert(query.clone());
        cache.insert(candidate);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &MediaExtractionSettings::fast_anchor_v2(),
        )
        .expect("anchor candidates should load");

        assert!(
            candidates.contains(&candidate_path),
            "candidate should be shortlisted when any video LSH band matches and full hash distance is within threshold"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_anchor_candidates_fallback_for_hamming_near_hashes_that_miss_lsh_bands() {
        let root = unique_media_match_test_root("video-lsh-fallback-candidates");
        let query_hash = 0x0123_4567_89ab_cdef;
        let candidate_hash = query_hash ^ 0x0001_0001_0001_0001;
        let mut query = fake_media_match_record("query.mkv");
        query.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        query.video = Some(sorotte_media_match::VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![sorotte_media_match::FrameFingerprint {
                timestamp_millis: 30_000,
                hash: query_hash,
            }],
        });
        let mut candidate = fake_media_match_record("candidate.mkv");
        candidate.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        candidate.video = Some(sorotte_media_match::VideoFingerprint {
            duration_seconds: Some(120),
            frames: vec![sorotte_media_match::FrameFingerprint {
                timestamp_millis: 31_000,
                hash: candidate_hash,
            }],
        });
        let candidate_path = candidate.identity.normalized_path.clone();
        let mut cache = MediaMatchCache::default();
        cache.insert(query.clone());
        cache.insert(candidate);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &MediaExtractionSettings::fast_anchor_v2(),
        )
        .expect("anchor candidates should load");

        assert!(
            candidates.contains(&candidate_path),
            "candidate should be shortlisted by Hamming fallback when all LSH bands differ"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_legacy_sqlite_tables_are_discarded_not_migrated() {
        let root = unique_media_match_test_root("legacy-drop");
        let index_dir = managed_media_match_index_dir(&root);
        std::fs::create_dir_all(&index_dir).expect("index dir should be created");
        let connection = Connection::open(managed_media_match_index_path(&root))
            .expect("legacy SQLite index should open");
        connection
            .execute_batch(
                "
                CREATE TABLE fingerprints (
                    normalized_path TEXT NOT NULL,
                    profile TEXT NOT NULL,
                    modified_unix_millis INTEGER NOT NULL,
                    size_bytes INTEGER NOT NULL,
                    algorithm_version INTEGER NOT NULL,
                    extraction_settings_json TEXT NOT NULL,
                    duration_seconds REAL,
                    record_json TEXT NOT NULL,
                    updated_unix_millis INTEGER NOT NULL,
                    PRIMARY KEY (normalized_path, profile)
                );
                INSERT INTO fingerprints (
                    normalized_path,
                    profile,
                    modified_unix_millis,
                    size_bytes,
                    algorithm_version,
                    extraction_settings_json,
                    duration_seconds,
                    record_json,
                    updated_unix_millis
                ) VALUES (
                    'episode.mkv',
                    'legacy',
                    1,
                    2,
                    1,
                    '{}',
                    120.0,
                    '{}',
                    3
                );
                CREATE TABLE fingerprints_v1 (record_json TEXT NOT NULL);
                INSERT INTO fingerprints_v1 VALUES ('{}');
                ",
            )
            .expect("legacy tables should be created");
        drop(connection);

        let connection = open_media_match_sqlite_index(&root).expect("SQLite index should reopen");
        let rows = connection
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("new fingerprint table should be readable");
        let legacy_exists = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'fingerprints_v1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("legacy table existence should be readable");

        assert_eq!(rows, 0);
        assert_eq!(legacy_exists, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_fast_v2_summary_storage_stays_under_two_kb_per_file() {
        let root = unique_media_match_test_root("fast-v2-size");
        let mut record = fake_media_match_record("episode.mkv");
        record.extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        record.audio_anchors = (0..96)
            .map(|index| AudioAnchor {
                bucket: index,
                t_ms: index * 10_000,
                weight: 1,
            })
            .collect();
        record.video_anchors = (0..48)
            .map(|index| VideoAnchor {
                bucket: index + 1_000,
                t_ms: index * 20_000,
                hash64: u64::from(index) << 32 | u64::from(index),
                weight: 1,
            })
            .collect();
        let mut cache = MediaMatchCache::default();
        cache.insert(record);
        save_media_match_cache(&root, &cache).expect("cache should be written");
        let connection = open_media_match_sqlite_index(&root).expect("SQLite index should open");
        let summary_bytes = connection
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(LENGTH(audio_summary), 0) + COALESCE(LENGTH(video_summary), 0)), 0)
                 FROM fingerprints
                 WHERE version = ?1 AND profile = ?2",
                params![
                    i64::from(MEDIA_MATCH_ANCHOR_VERSION),
                    MediaFingerprintProfile::FastAnchorV2.label(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .expect("summary byte count should load");

        assert!(
            summary_bytes <= 2_048,
            "fast profile summary bytes should stay under 2KB, got {summary_bytes}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_initial_scan_inventories_without_fingerprinting_when_no_current_file() {
        let root = unique_media_match_test_root("inventory-only");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        std::fs::write(media_dir.join("episode-01.mkv"), b"one")
            .expect("first media placeholder should be written");
        std::fs::write(media_dir.join("episode-02.mkv"), b"two")
            .expect("second media placeholder should be written");
        let settings = MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        };
        let result = rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
            &root,
            std::slice::from_ref(&media_dir),
            None,
            &settings,
            &MediaExtractionSettings::fast_anchor_v2(),
            None,
            |_| {},
        )
        .expect("inventory-only scan should not require media tools");
        let connection = open_media_match_sqlite_index(&root).expect("SQLite index should open");
        let inventory = connection
            .query_row("SELECT COUNT(*) FROM media_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("inventory count should load");
        let fingerprints = connection
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("fingerprint count should load");

        assert_eq!(inventory, 2);
        assert_eq!(fingerprints, 0);
        assert!(result.message.contains("inventoried 2 discovered files"));
        assert!(result.message.contains("No active local media path"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_summary_reports_nearest_reject_with_debug_metrics() {
        let root = unique_media_match_test_root("nearest-reject");
        let mut query = fake_media_match_record("episode-current.mkv");
        query.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 10_000,
            weight: 1,
        }];
        let mut nearest = fake_media_match_record("episode-nearest.mkv");
        nearest.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 12_000,
            weight: 1,
        }];
        let mut unrelated = fake_media_match_record("episode-unrelated.mkv");
        unrelated.audio_anchors = vec![AudioAnchor {
            bucket: 84,
            t_ms: 10_000,
            weight: 1,
        }];
        let mut cache = MediaMatchCache::default();
        cache.insert(query);
        cache.insert(unrelated);
        cache.insert(nearest);

        let (current_decision, nearest_match, last_evidence) = summarize_current_media_match(
            &root,
            Some("episode-current.mkv"),
            &cache,
            &enabled_media_match_settings(),
            &MediaExtractionSettings::fast_anchor_v2(),
        );

        assert_eq!(
            current_decision.as_deref(),
            Some("exact: current local file is indexed")
        );
        let nearest_match = nearest_match.expect("nearest candidate should be reported");
        assert!(
            nearest_match.contains(
                "No alternate indexed match; nearest other: episode-nearest.mkv (reject: anchor timeline evidence is insufficient)"
            ),
            "{nearest_match}"
        );
        let last_evidence = last_evidence.expect("debug evidence should be reported");
        assert!(
            last_evidence.contains("current file is indexed exactly"),
            "{last_evidence}"
        );
        assert!(last_evidence.contains("tier=reject"), "{last_evidence}");
        assert!(
            last_evidence.contains("alignment offset=2.0s"),
            "{last_evidence}"
        );
        assert!(last_evidence.contains("pairs=1"), "{last_evidence}");
        assert!(last_evidence.contains("audio=1"), "{last_evidence}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_rebuild_with_no_fresh_work_does_not_rewrite_sqlite_records() {
        let root = unique_media_match_test_root("no-op-rebuild");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let media_path = media_dir.join("episode.mkv");
        std::fs::write(&media_path, vec![42u8; 2000]).expect("test media file should be written");
        let extraction_settings = MediaExtractionSettings::fast_anchor_v2();
        let record = fake_media_match_record_for_file(&media_path, extraction_settings.clone());
        let mut cache = MediaMatchCache::default();
        cache.insert(record.clone());
        save_media_match_cache(&root, &cache).expect("cache should be written");
        let before = media_match_record_updated_unix_millis(&root, &record);
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut settings = MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        };
        settings.runtime_tolerance_enabled = true;
        let tools = MediaMatchToolPaths {
            ffmpeg: PathBuf::from("ffmpeg-not-used"),
            ffprobe: PathBuf::from("ffprobe-not-used"),
            fpcalc: PathBuf::from("fpcalc-not-used"),
        };
        let result = rebuild_persisted_media_match_candidates_with_progress_and_cancel(
            MediaMatchCandidateRebuildRequest {
                root: &root,
                candidates: vec![media_path],
                current_player_path: None,
                settings: &settings,
                tools: &tools,
                extraction_settings: &extraction_settings,
                cancel_flag: None,
            },
            |_| {},
        )
        .expect("no-op rebuild should succeed without invoking tools");
        let after = media_match_record_updated_unix_millis(&root, &record);

        assert_eq!(before, after);
        assert!(result.message.contains("already current"));
        assert!(
            result
                .cache_status
                .starts_with("inventory: 1, fast(all settings): 1, full(all settings): 0")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_rebuild_cancellation_does_not_checkpoint_partial_fingerprint() {
        let root = unique_media_match_test_root("cancel-no-checkpoint");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let media_path = media_dir.join("episode.mkv");
        std::fs::write(&media_path, vec![42u8; 2000]).expect("test media file should be written");
        let settings = MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        };
        let tools = MediaMatchToolPaths {
            ffmpeg: PathBuf::from("ffmpeg-not-used"),
            ffprobe: PathBuf::from("ffprobe-not-used"),
            fpcalc: PathBuf::from("fpcalc-not-used"),
        };
        let cancel = AtomicBool::new(true);
        let result = rebuild_persisted_media_match_candidates_with_progress_and_cancel(
            MediaMatchCandidateRebuildRequest {
                root: &root,
                candidates: vec![media_path.clone()],
                current_player_path: media_path.to_str(),
                settings: &settings,
                tools: &tools,
                extraction_settings: &MediaExtractionSettings::fast_anchor_v2(),
                cancel_flag: Some(&cancel),
            },
            |_| {},
        );

        assert!(result.is_err());
        let connection = open_media_match_sqlite_index(&root).expect("SQLite index should open");
        let fingerprints = connection
            .query_row("SELECT COUNT(*) FROM fingerprints", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("fingerprint count should load");
        assert_eq!(
            fingerprints, 0,
            "cancelled rebuilds must not checkpoint empty or partial fingerprints"
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
        let mut existing = MediaMatchCache::default();
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
