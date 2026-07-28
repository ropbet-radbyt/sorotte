use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use std::io::Read;

use serde::{Deserialize, Serialize};
use sorotte_media_match::{
    MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFingerprintError,
    MediaFingerprintRecord, MediaIndexBuildTransaction, MediaIndexCommitError,
    MediaIndexCommitOutcome, MediaIndexInventoryEntry, MediaIndexService, MediaIndexSession,
    MediaMatchCache, MediaMatchCandidateDecision, MediaMatchDecision, MediaMatchSettings,
    MediaMatchTier, MediaMatchToolPaths, MediaMatchV3RetrievalStats, decide_media_match,
    fingerprint_media_file_cancellable_with_report, media_extraction_settings_hash,
    media_match_wire_value_from_records, normalize_media_path, rank_media_match_candidates,
    summarize_record_v3_diagnostics,
};

#[cfg(test)]
use sorotte_media_match::{AudioAnchor, map_query_position_to_candidate_ms};

use super::shell_state::{
    GuiMediaMatchRuntimeSnapshot, GuiMediaMatchToolHealth,
    media_match_settings_from_stored_settings,
};

#[cfg(windows)]
use zip::ZipArchive;

const MEDIA_MATCH_METADATA_VERSION: u32 = 1;
const MEDIA_MATCH_PREFILTER_THRESHOLD: usize = 64;
const MEDIA_MATCH_PREFILTER_LIMIT: usize = 24;
const MEDIA_MATCH_DISCOVERY_MAX_DEPTH: usize = 64;
const MEDIA_MATCH_DISCOVERY_MAX_NODES: usize = 250_000;
const MEDIA_MATCH_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MEDIA_MATCH_VERSION_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(25);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaMatchTool {
    Ffmpeg,
    Ffprobe,
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

pub(super) struct MediaMatchIndexRebuildRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) tool_root: &'a Path,
    pub(super) search_roots: &'a [PathBuf],
    pub(super) current_player_path: Option<&'a str>,
    pub(super) settings: &'a MediaMatchSettings,
    pub(super) extraction_settings: &'a MediaExtractionSettings,
    pub(super) cancel_flag: Option<&'a AtomicBool>,
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

pub(super) struct MediaMatchRemoteCandidateRebuildRequest<'a> {
    pub(super) root: &'a Path,
    pub(super) search_roots: &'a [PathBuf],
    pub(super) candidates: Option<Vec<PathBuf>>,
    pub(super) target_file_name: &'a str,
    pub(super) media_match_signature: &'a sorotte_media_match::MediaMatchWireSignature,
    pub(super) settings: &'a MediaMatchSettings,
    pub(super) tools: &'a MediaMatchToolPaths,
    pub(super) extraction_settings: &'a MediaExtractionSettings,
    pub(super) cancel_flag: Option<&'a AtomicBool>,
}

#[derive(Debug, Clone)]
pub(super) struct MediaMatchRemoteCandidateMatch {
    pub(super) path: String,
    pub(super) decision: MediaMatchDecision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MediaMatchRebuildInstrumentation {
    ffmpeg_invocations: u32,
    ffprobe_invocations: u32,
    extraction_millis: u128,
    ffprobe_millis: u128,
    audio_millis: u128,
    audio_streamed_bytes: usize,
    audio_streamed_samples: usize,
    audio_stream_peak_frames: usize,
    audio_stream_raw_landmarks: usize,
    audio_stream_raw_landmarks_emitted: usize,
    audio_stream_final_landmarks: usize,
    max_audio_stream_buffer_samples: usize,
    max_audio_stream_raw_landmarks_seen: usize,
    max_audio_stream_raw_landmarks_after_compaction: usize,
    audio_stream_raw_compactions: usize,
    audio_stream_pcm_drain_millis: u128,
    audio_stream_analyzer_millis: u128,
    audio_stream_backpressure_millis: u128,
    max_audio_stream_queued_pcm_bytes: usize,
    audio_stream_candidate_pairs_considered: usize,
    audio_stream_landmarks_accepted: usize,
    audio_stream_landmarks_rejected: usize,
    debug_record_bytes: usize,
    audio_blob_bytes: usize,
    audio_index_rows: usize,
    stats_refreshes: u32,
    stats_refresh_millis: u128,
    background_index_worker_count: usize,
    sampled_fast_worker_count: usize,
    extraction_queue_wait_millis: u128,
    extraction_worker_wall_millis: u128,
    sqlite_writer_millis: u128,
    files_indexed: usize,
    cancelled_file_count: usize,
    resumed_file_count: usize,
}

impl MediaMatchRebuildInstrumentation {
    fn add_report(&mut self, report: &sorotte_media_match::MediaFingerprintExtractionReport) {
        self.ffmpeg_invocations += report.invocations.ffmpeg;
        self.ffprobe_invocations += report.invocations.ffprobe;
        self.extraction_millis += report.timings.total_millis;
        self.ffprobe_millis += report.timings.ffprobe_millis;
        self.audio_millis += report.timings.audio_millis;
        self.audio_streamed_bytes += report.audio_stream.streamed_bytes;
        self.audio_streamed_samples += report.audio_stream.streamed_samples;
        self.audio_stream_peak_frames += report.audio_stream.peak_frames;
        self.audio_stream_raw_landmarks_emitted += report.audio_stream.raw_landmarks_emitted;
        self.audio_stream_raw_landmarks += report.audio_stream.raw_landmarks_before_bounding;
        self.audio_stream_final_landmarks += report.audio_stream.final_landmarks;
        self.max_audio_stream_buffer_samples = self
            .max_audio_stream_buffer_samples
            .max(report.audio_stream.max_buffer_samples);
        self.max_audio_stream_raw_landmarks_seen = self
            .max_audio_stream_raw_landmarks_seen
            .max(report.audio_stream.max_raw_landmarks_seen);
        self.max_audio_stream_raw_landmarks_after_compaction = self
            .max_audio_stream_raw_landmarks_after_compaction
            .max(report.audio_stream.max_raw_landmarks_after_compaction);
        self.audio_stream_raw_compactions += report.audio_stream.raw_landmark_compactions;
        self.audio_stream_pcm_drain_millis += report.audio_stream.pcm_decode_drain_millis;
        self.audio_stream_analyzer_millis += report.audio_stream.analyzer_millis;
        self.audio_stream_candidate_pairs_considered +=
            report.audio_stream.candidate_pairs_considered;
        self.audio_stream_landmarks_accepted +=
            report.audio_stream.landmarks_accepted_into_reservoir;
        self.audio_stream_landmarks_rejected += report.audio_stream.landmarks_rejected_by_reservoir;
        self.debug_record_bytes += report.serialized_debug_record_bytes;
    }

    fn add_saved_record(&mut self, record: &MediaFingerprintRecord) {
        let diagnostics = summarize_record_v3_diagnostics(record);
        self.audio_blob_bytes += diagnostics.audio_blob_bytes;
        self.audio_index_rows += diagnostics.audio_index_count;
    }

    fn add_stats_refresh(&mut self, elapsed_millis: u128) {
        self.stats_refreshes += 1;
        self.stats_refresh_millis += elapsed_millis;
    }

    fn add_parallel_stats(&mut self, stats: &MediaMatchParallelExtractionStats) {
        self.background_index_worker_count = self
            .background_index_worker_count
            .max(stats.background_index_worker_count);
        self.sampled_fast_worker_count = self
            .sampled_fast_worker_count
            .max(stats.sampled_fast_worker_count);
        self.extraction_queue_wait_millis = self
            .extraction_queue_wait_millis
            .saturating_add(stats.extraction_queue_wait_millis);
        self.extraction_worker_wall_millis = self
            .extraction_worker_wall_millis
            .saturating_add(stats.extraction_worker_wall_millis);
        self.files_indexed = self.files_indexed.saturating_add(stats.files_indexed);
        self.cancelled_file_count = self
            .cancelled_file_count
            .saturating_add(stats.cancelled_file_count);
        self.resumed_file_count = self
            .resumed_file_count
            .saturating_add(stats.resumed_file_count);
    }

    fn add_sqlite_writer_millis(&mut self, elapsed_millis: u128) {
        self.sqlite_writer_millis = self.sqlite_writer_millis.saturating_add(elapsed_millis);
    }

    fn files_per_minute(&self) -> u64 {
        if self.files_indexed == 0 || self.extraction_worker_wall_millis == 0 {
            return 0;
        }
        let rounded = (self.files_indexed as u128)
            .saturating_mul(60_000)
            .saturating_add(self.extraction_worker_wall_millis / 2)
            / self.extraction_worker_wall_millis;
        rounded.min(u64::MAX as u128) as u64
    }

    fn summary(&self) -> String {
        format!(
            "tools ffmpeg/ffprobe={}/{}, extract={}ms (probe {}ms, audio {}ms), workers background/sampledFast={}/{}, queueWait={}ms workerWall={}ms sqliteWriter={}ms filesIndexed={} filesPerMinute={} cancelled={} resumed={}, v3 audio stream streamedBytes/streamedSamples/peakFrames/rawLandmarksEmitted/rawLandmarksBeforeBounding/finalLandmarks/maxBufferSamples/maxRawLandmarksSeen/maxRawLandmarksAfterCompaction/rawLandmarkCompactions/pcmDrainMillis/analyzerMillis/backpressureMillis/maxQueuedPcmBytes/candidatePairsConsidered/landmarksAccepted/landmarksRejected={}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}, v3 blob bytes audio={}, v3 index rows audio={}, stats refreshes={} in {}ms (debug record bytes={})",
            self.ffmpeg_invocations,
            self.ffprobe_invocations,
            self.extraction_millis,
            self.ffprobe_millis,
            self.audio_millis,
            self.background_index_worker_count,
            self.sampled_fast_worker_count,
            self.extraction_queue_wait_millis,
            self.extraction_worker_wall_millis,
            self.sqlite_writer_millis,
            self.files_indexed,
            self.files_per_minute(),
            self.cancelled_file_count,
            self.resumed_file_count,
            self.audio_streamed_bytes,
            self.audio_streamed_samples,
            self.audio_stream_peak_frames,
            self.audio_stream_raw_landmarks_emitted,
            self.audio_stream_raw_landmarks,
            self.audio_stream_final_landmarks,
            self.max_audio_stream_buffer_samples,
            self.max_audio_stream_raw_landmarks_seen,
            self.max_audio_stream_raw_landmarks_after_compaction,
            self.audio_stream_raw_compactions,
            self.audio_stream_pcm_drain_millis,
            self.audio_stream_analyzer_millis,
            self.audio_stream_backpressure_millis,
            self.max_audio_stream_queued_pcm_bytes,
            self.audio_stream_candidate_pairs_considered,
            self.audio_stream_landmarks_accepted,
            self.audio_stream_landmarks_rejected,
            self.audio_blob_bytes,
            self.audio_index_rows,
            self.stats_refreshes,
            self.stats_refresh_millis,
            self.debug_record_bytes
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MediaMatchParallelExtractionStats {
    background_index_worker_count: usize,
    sampled_fast_worker_count: usize,
    extraction_queue_wait_millis: u128,
    extraction_worker_wall_millis: u128,
    files_indexed: usize,
    cancelled_file_count: usize,
    resumed_file_count: usize,
}

type MediaMatchParallelExtractionResult = Result<
    (
        MediaFingerprintRecord,
        sorotte_media_match::MediaFingerprintExtractionReport,
    ),
    MediaFingerprintError,
>;

#[derive(Debug)]
struct MediaMatchParallelExtractionOutput {
    path: PathBuf,
    queue_wait_millis: u128,
    worker_wall_millis: u128,
    result: MediaMatchParallelExtractionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct ManagedMediaMatchMetadata {
    version: u32,
    installed_at_unix_seconds: Option<u64>,
    ffmpeg_version: Option<String>,
    ffprobe_version: Option<String>,
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
        }
    }

    fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg | Self::Ffprobe => &["-version"],
        }
    }

    fn assign_version(self, metadata: &mut ManagedMediaMatchMetadata, version: String) {
        match self {
            Self::Ffmpeg => metadata.ffmpeg_version = Some(version),
            Self::Ffprobe => metadata.ffprobe_version = Some(version),
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
    MediaIndexService::new(managed_media_match_index_dir(root)).index_path()
}

pub(super) fn media_match_sqlite_index_exists(root: &Path) -> bool {
    managed_media_match_index_path(root).exists()
}

#[derive(Debug)]
pub(super) struct GuiMediaMatchIndexBuildTransaction {
    staging_app_root: PathBuf,
    transaction: Option<MediaIndexBuildTransaction>,
}

impl GuiMediaMatchIndexBuildTransaction {
    pub(super) fn staging_app_root(&self) -> &Path {
        &self.staging_app_root
    }

    pub(super) fn commit(mut self) -> Result<MediaIndexCommitOutcome, MediaIndexCommitError> {
        let result = self
            .transaction
            .take()
            .expect("media-match build transaction should be present")
            .commit();
        let outer_cleanup = if self.staging_app_root.exists() {
            fs::remove_dir_all(&self.staging_app_root)
                .err()
                .map(|error| {
                    format!(
                        "failed removing media-match staging directory '{}': {error}",
                        self.staging_app_root.display()
                    )
                })
        } else {
            None
        };
        match (result, outer_cleanup) {
            (Ok(MediaIndexCommitOutcome::Activated { cleanup_warning }), outer_cleanup) => {
                let warnings = [cleanup_warning, outer_cleanup]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                Ok(MediaIndexCommitOutcome::Activated {
                    cleanup_warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
                })
            }
            (Err(MediaIndexCommitError::NotActivated(error)), cleanup) => {
                Err(MediaIndexCommitError::NotActivated(match cleanup {
                    Some(cleanup) => format!("{error}; {cleanup}"),
                    None => error,
                }))
            }
            (Err(MediaIndexCommitError::StaleBase(error)), cleanup) => {
                Err(MediaIndexCommitError::StaleBase(match cleanup {
                    Some(cleanup) => format!("{error}; {cleanup}"),
                    None => error,
                }))
            }
        }
    }

    pub(super) fn abort(mut self) -> Result<(), String> {
        self.transaction
            .take()
            .expect("media-match build transaction should be present")
            .abort()?;
        if self.staging_app_root.exists() {
            fs::remove_dir_all(&self.staging_app_root).map_err(|error| {
                format!(
                    "failed removing media-match staging directory '{}': {error}",
                    self.staging_app_root.display()
                )
            })?;
        }
        Ok(())
    }
}

pub(super) fn prepare_media_match_index_rebuild_backup(
    root: &Path,
) -> Result<GuiMediaMatchIndexBuildTransaction, String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging_app_root = root.join("cache").join(format!(
        ".media-match-build-{}-{unique}",
        std::process::id()
    ));
    let transaction = MediaIndexBuildTransaction::begin(
        managed_media_match_index_dir(root),
        managed_media_match_index_dir(&staging_app_root),
    )?;
    Ok(GuiMediaMatchIndexBuildTransaction {
        staging_app_root,
        transaction: Some(transaction),
    })
}

pub(super) fn clear_persisted_media_match_cache_at_root(root: &Path) -> Result<(), String> {
    let index_dir = managed_media_match_index_dir(root);
    if index_dir.exists() {
        fs::remove_dir_all(&index_dir).map_err(|error| {
            format!(
                "failed removing media-match SQLite index directory '{}': {error}",
                index_dir.display()
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
    let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
    let ffmpeg = probe_tool(root, MediaMatchTool::Ffmpeg);
    let ffprobe = probe_tool(root, MediaMatchTool::Ffprobe);
    media_match_runtime_snapshot_from_probes(root, settings, ffmpeg, ffprobe, &extraction_settings)
}

fn media_match_runtime_snapshot_from_probes(
    root: Option<&Path>,
    settings: &MediaMatchSettings,
    ffmpeg: MediaMatchToolProbe,
    ffprobe: MediaMatchToolProbe,
    extraction_settings: &MediaExtractionSettings,
) -> GuiMediaMatchRuntimeSnapshot {
    let health = media_match_health_for_settings(&ffmpeg, &ffprobe, extraction_settings);
    let message = media_match_health_message(health, &ffmpeg, &ffprobe);
    GuiMediaMatchRuntimeSnapshot {
        settings: settings.clone(),
        health,
        message,
        install_supported: cfg!(windows),
        integration_supported: true,
        install_location: root.map(|root| managed_media_match_bin_dir(root).display().to_string()),
        ffmpeg_status: Some(ffmpeg.status),
        ffprobe_status: Some(ffprobe.status),
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
            Some("Import ffmpeg and ffprobe manually on this platform.".to_owned()),
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
        let mut metadata = ManagedMediaMatchMetadata {
            version: MEDIA_MATCH_METADATA_VERSION,
            installed_at_unix_seconds: Some(current_unix_seconds()),
            ..ManagedMediaMatchMetadata::default()
        };
        for tool in [MediaMatchTool::Ffmpeg, MediaMatchTool::Ffprobe] {
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
            Some(format!("{}; V3 is ready.", bin_dir.display())),
            1.0,
        ));
        Ok(media_match_install_success_message())
    }
}

#[cfg(any(windows, test))]
fn media_match_install_success_message() -> String {
    "Installed ffmpeg and ffprobe for Media Matching V3.".to_owned()
}

#[cfg(test)]
pub(super) fn rebuild_persisted_media_match_index_with_extraction_settings_and_cancel<F>(
    root: &Path,
    search_roots: &[PathBuf],
    current_player_path: Option<&str>,
    settings: &MediaMatchSettings,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
    progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    rebuild_persisted_media_match_index_with_tool_root_and_cancel(
        MediaMatchIndexRebuildRequest {
            root,
            tool_root: root,
            search_roots,
            current_player_path,
            settings,
            extraction_settings,
            cancel_flag,
        },
        progress,
    )
}

pub(super) fn rebuild_persisted_media_match_index_with_tool_root_and_cancel<F>(
    request: MediaMatchIndexRebuildRequest<'_>,
    mut progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    let MediaMatchIndexRebuildRequest {
        root,
        tool_root,
        search_roots,
        current_player_path,
        settings,
        extraction_settings,
        cancel_flag,
    } = request;
    if !settings.fingerprinting_enabled {
        return Err("Enable Media Matching fingerprinting before rebuilding the index.".to_owned());
    }
    progress(MediaMatchToolProgress::new(
        "Scanning media-search roots",
        Some(format!("{} roots", search_roots.len())),
        0.05,
    ));
    let discovery = discover_media_match_candidates(search_roots, cancel_flag)?;
    let candidates = discovery.candidates;
    if current_player_path.is_none() {
        inventory_media_match_candidates(root, &discovery.scanned_roots, &candidates, cancel_flag)?;
        match media_match_tool_paths_for_settings(tool_root, extraction_settings) {
            Ok(tools) => {
                return rebuild_persisted_media_match_candidates_with_progress_and_cancel(
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
                );
            }
            Err(error) => {
                let cache_status = media_match_cache_status(root);
                progress(MediaMatchToolProgress::new(
                    "Media Matching inventory updated",
                    Some(cache_status.clone()),
                    1.0,
                ));
                return Ok(MediaMatchIndexRebuildResult {
                    message: format!(
                        "Media Matching inventoried {} discovered files. No active local media path could be resolved and sampled-fast indexing is waiting for Media Matching tools: {error}",
                        candidates.len()
                    ),
                    cache_status,
                    current_decision: Some("unknown: no resolved current local file".to_owned()),
                    nearest_match: None,
                    last_evidence: None,
                });
            }
        }
    }
    let tools = media_match_tool_paths_for_settings(tool_root, extraction_settings)?;
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

    let checkpoint_session = open_media_match_index_session(request.root)?;
    let mut next_cache = initial_media_match_rebuild_cache(&existing_cache, selected.prefiltered);
    let mut instrumentation = MediaMatchRebuildInstrumentation::default();
    let mut parallel_fresh_results = BTreeMap::new();
    let mut parallel_fresh_work_done = 0usize;
    if fresh_work_total > 1 {
        let fresh_paths = selected
            .paths
            .iter()
            .filter(|path| {
                !media_match_cache_has_valid_record(
                    &existing_cache,
                    path,
                    request.extraction_settings,
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let (results, stats) = parallel_fresh_media_fingerprints(
            fresh_paths,
            request.tools,
            request.extraction_settings,
            request.cancel_flag,
            |completed, path| {
                parallel_fresh_work_done = completed;
                let progress_fraction =
                    0.1 + (0.82 * (completed as f32 / fresh_work_total.max(1) as f32));
                progress(MediaMatchToolProgress::new(
                    "Fingerprinting media",
                    Some(format!(
                        "{completed}/{fresh_work_total} files needing index: {}",
                        path.display()
                    )),
                    progress_fraction,
                ));
            },
        )?;
        instrumentation.add_parallel_stats(&stats);
        parallel_fresh_results = results;
        fresh_work_done = parallel_fresh_work_done;
    }

    for (index, path) in selected.paths.iter().enumerate() {
        let normalized_path = normalize_media_path(path);
        let has_parallel_result = parallel_fresh_results.contains_key(&normalized_path);
        let denominator = total.max(1);
        let parallel_prefetched = parallel_fresh_work_done > 0;
        let progress_fraction = if parallel_prefetched {
            0.92 + (0.06 * (index as f32 / denominator as f32))
        } else {
            0.1 + (0.82 * (index as f32 / denominator as f32))
        };
        let path_needs_fingerprint =
            !media_match_cache_has_valid_record(&existing_cache, path, request.extraction_settings);
        if request
            .cancel_flag
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
            && path_needs_fingerprint
            && !has_parallel_result
        {
            skipped += 1;
            fresh_work_done += 1;
            continue;
        }
        progress(MediaMatchToolProgress::new(
            if parallel_prefetched {
                "Saving media fingerprints"
            } else {
                "Fingerprinting media"
            },
            Some(format!(
                "{fresh_work_done}/{fresh_work_total} files needing index: {}",
                path.display()
            )),
            progress_fraction,
        ));
        let parallel_result = path_needs_fingerprint
            .then(|| parallel_fresh_results.remove(&normalized_path))
            .flatten();
        let from_parallel_result = parallel_result.is_some();
        let fingerprint_result = if path_needs_fingerprint {
            if let Some(result) = parallel_result {
                result.map(|(record, report)| (record, false, Some(report)))
            } else {
                cached_or_fresh_media_fingerprint(
                    &existing_cache,
                    path,
                    request.tools,
                    request.extraction_settings,
                    request.cancel_flag,
                )
            }
        } else {
            cached_or_fresh_media_fingerprint(
                &existing_cache,
                path,
                request.tools,
                request.extraction_settings,
                request.cancel_flag,
            )
        };
        match fingerprint_result {
            Ok((record, was_reused, report)) => {
                if was_reused {
                    reused += 1;
                } else {
                    fingerprinted += 1;
                    if !from_parallel_result {
                        fresh_work_done += 1;
                    }
                    let sqlite_started_at = Instant::now();
                    checkpoint_session.save_record(&record, None)?;
                    instrumentation
                        .add_sqlite_writer_millis(sqlite_started_at.elapsed().as_millis());
                    if let Some(report) = report {
                        instrumentation.add_report(&report);
                    }
                    instrumentation.add_saved_record(&record);
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
                if from_parallel_result {
                    skipped += 1;
                } else {
                    return Err("Media Matching index rebuild was canceled.".to_owned());
                }
            }
            Err(_) => {
                skipped += 1;
                if path_needs_fingerprint && !from_parallel_result {
                    fresh_work_done += 1;
                }
            }
        }
    }

    if request
        .cancel_flag
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        return Err("Media Matching index rebuild was canceled.".to_owned());
    }

    refresh_media_match_v3_anchor_stats_for_settings(
        &checkpoint_session,
        request.extraction_settings,
        &mut instrumentation,
    )?;

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

pub(super) fn rebuild_persisted_media_match_remote_candidates_with_progress_and_cancel<F>(
    request: MediaMatchRemoteCandidateRebuildRequest<'_>,
    mut progress: F,
) -> Result<MediaMatchIndexRebuildResult, String>
where
    F: FnMut(MediaMatchToolProgress),
{
    let signature = request.media_match_signature;
    progress(MediaMatchToolProgress::new(
        "Scanning media-search roots",
        Some(format!("{} roots", request.search_roots.len())),
        0.05,
    ));
    let (candidates, scanned_roots) = match request.candidates.as_ref() {
        Some(candidates) => (candidates.clone(), None),
        None => {
            let discovery =
                discover_media_match_candidates(request.search_roots, request.cancel_flag)?;
            (discovery.candidates, Some(discovery.scanned_roots))
        }
    };
    if let Some(scanned_roots) = scanned_roots.as_ref() {
        inventory_media_match_candidates(
            request.root,
            scanned_roots,
            &candidates,
            request.cancel_flag,
        )?;
    }
    let selected = select_remote_media_match_candidates(&candidates, request.target_file_name);
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
    let mut best_match = best_remote_candidate_match(
        &selected.paths,
        &existing_cache,
        signature,
        request.settings,
        request.extraction_settings,
    );
    let mut next_cache = existing_cache.clone();
    let checkpoint_session = open_media_match_index_session(request.root)?;
    let mut instrumentation = MediaMatchRebuildInstrumentation::default();

    progress(MediaMatchToolProgress::new(
        "Fingerprinting media",
        Some(format!(
            "0/{fresh_work_total} room-candidate files needing index"
        )),
        0.1,
    ));

    for (index, path) in selected.paths.iter().enumerate() {
        if request
            .cancel_flag
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err("Media Matching room-candidate rebuild was canceled.".to_owned());
        }
        if best_match
            .as_ref()
            .is_some_and(|best| media_match_tier_is_strong_or_exact(best.decision.tier))
        {
            break;
        }
        let denominator = total.max(1);
        let progress_fraction = 0.1 + (0.82 * (index as f32 / denominator as f32));
        let path_needs_fingerprint =
            !media_match_cache_has_valid_record(&existing_cache, path, request.extraction_settings);
        progress(MediaMatchToolProgress::new(
            "Fingerprinting media",
            Some(format!(
                "{fresh_work_done}/{fresh_work_total} room-candidate files needing index: {}",
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
                    checkpoint_session.save_record(&record, None)?;
                    if let Some(report) = report {
                        instrumentation.add_report(&report);
                    }
                    instrumentation.add_saved_record(&record);
                }
                let decision = sorotte_media_match::decide_media_match_against_wire_signature(
                    &record,
                    signature,
                    request.settings,
                );
                if best_match
                    .as_ref()
                    .is_none_or(|best| media_match_decision_is_better(&decision, &best.decision))
                {
                    best_match = Some(MediaMatchRemoteCandidateMatch {
                        path: record.identity.normalized_path.clone(),
                        decision,
                    });
                }
                next_cache.insert(record);
            }
            Err(MediaFingerprintError::Cancelled { .. }) => {
                return Err("Media Matching room-candidate rebuild was canceled.".to_owned());
            }
            Err(_) => {
                skipped += 1;
                if path_needs_fingerprint {
                    fresh_work_done += 1;
                }
            }
        }
    }

    if best_match
        .as_ref()
        .is_none_or(|best| !media_match_tier_is_strong_or_exact(best.decision.tier))
    {
        best_match = best_remote_candidate_match(
            &selected.paths,
            &next_cache,
            signature,
            request.settings,
            request.extraction_settings,
        );
    }

    refresh_media_match_v3_anchor_stats_for_settings(
        &checkpoint_session,
        request.extraction_settings,
        &mut instrumentation,
    )?;

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
    progress(MediaMatchToolProgress::new(
        "Media Matching room candidates indexed",
        Some(format!(
            "{fresh_work_done}/{fresh_work_total} room-candidate files needing index; {cache_status}"
        )),
        1.0,
    ));

    let (message, current_decision, nearest_match, last_evidence) = if let Some(best) = best_match
        .as_ref()
        .filter(|best| media_match_tier_is_strong_or_exact(best.decision.tier))
    {
        let tier = media_match_tier_label(best.decision.tier);
        (
            format!(
                "Media Matching indexed room candidates across {scope} ({} reused, {} fingerprinted, {} skipped; {}).",
                reused,
                fingerprinted,
                skipped,
                instrumentation.summary()
            ),
            Some(format!("{tier}: room media matched {}", best.path)),
            Some(format!(
                "{} ({tier}: {})",
                best.path, best.decision.explanation
            )),
            Some(format_media_match_evidence_summary(&best.decision)),
        )
    } else if let Some(best) = best_match {
        let tier = media_match_tier_label(best.decision.tier);
        let current_decision = if best.decision.tier == MediaMatchTier::Probable {
            format!("{tier}: room media candidate found; sampled-only matches do not autoplay")
        } else {
            "unknown: no strong local match for room media yet".to_owned()
        };
        (
            format!(
                "Media Matching indexed room candidates across {scope} ({} reused, {} fingerprinted, {} skipped; {}).",
                reused,
                fingerprinted,
                skipped,
                instrumentation.summary()
            ),
            Some(current_decision),
            Some(format!(
                "Nearest local room candidate: {} ({})",
                best.path, best.decision.explanation
            )),
            Some(format_media_match_evidence_summary(&best.decision)),
        )
    } else {
        (
            format!(
                "Media Matching indexed room candidates across {scope} ({} reused, {} fingerprinted, {} skipped; {}).",
                reused,
                fingerprinted,
                skipped,
                instrumentation.summary()
            ),
            Some("unknown: no strong local match for room media yet".to_owned()),
            None,
            None,
        )
    };

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

fn media_match_health_for_settings(
    ffmpeg: &MediaMatchToolProbe,
    ffprobe: &MediaMatchToolProbe,
    _extraction_settings: &MediaExtractionSettings,
) -> GuiMediaMatchToolHealth {
    if ffmpeg.error.is_some() || ffprobe.error.is_some() {
        return GuiMediaMatchToolHealth::Broken;
    }
    if ffmpeg.path.is_none() {
        return GuiMediaMatchToolHealth::MissingFfmpeg;
    }
    if ffprobe.path.is_none() {
        return GuiMediaMatchToolHealth::MissingFfprobe;
    }
    GuiMediaMatchToolHealth::Healthy
}

fn media_match_health_message(
    health: GuiMediaMatchToolHealth,
    ffmpeg: &MediaMatchToolProbe,
    ffprobe: &MediaMatchToolProbe,
) -> Option<String> {
    match health {
        GuiMediaMatchToolHealth::Healthy => None,
        GuiMediaMatchToolHealth::MissingFfmpeg => {
            Some("Media Matching needs ffmpeg for audio decoding.".to_owned())
        }
        GuiMediaMatchToolHealth::MissingFfprobe => {
            Some("Media Matching needs ffprobe for media metadata.".to_owned())
        }
        GuiMediaMatchToolHealth::Broken => Some(format!(
            "One or more Media Matching tools could not run: {}; {}",
            ffmpeg.status, ffprobe.status
        )),
    }
}

fn media_match_cache_status(root: &Path) -> String {
    if !managed_media_match_index_path(root).exists() {
        return "empty".to_owned();
    }
    let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
    match open_media_match_index_session(root).and_then(|session| session.summary(&settings)) {
        Ok(summary)
            if summary.inventory_count > 0 || summary.fixed_settings_fingerprint_count > 0 =>
        {
            let storage = if summary.v3_fingerprint_row_count == 0 {
                String::new()
            } else {
                let average =
                    summary.v3_audio_blob_bytes as f64 / summary.v3_fingerprint_row_count as f64;
                format!(
                    "db={} bytes, v3 audio blobs: {} bytes ({average:.0}/fingerprint), verify audio={}, index audio={}",
                    summary.database_bytes,
                    summary.v3_audio_blob_bytes,
                    summary.v3_audio_verify_count,
                    summary.v3_audio_index_count,
                )
            };
            let active = format!(
                "active settings audio={}",
                summary.current_settings_fingerprint_count
            );
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
                "inventory: {}, audio-v3(all settings): {}{details}",
                summary.inventory_count, summary.fixed_settings_fingerprint_count
            )
        }
        Ok(_) => "empty".to_owned(),
        Err(error) => format!("unreadable cache: {error}"),
    }
}

pub(super) fn media_match_tool_paths_for_settings(
    root: &Path,
    extraction_settings: &MediaExtractionSettings,
) -> Result<MediaMatchToolPaths, String> {
    let ffmpeg = probe_tool(Some(root), MediaMatchTool::Ffmpeg);
    let ffprobe = probe_tool(Some(root), MediaMatchTool::Ffprobe);
    let health = media_match_health_for_settings(&ffmpeg, &ffprobe, extraction_settings);
    if health != GuiMediaMatchToolHealth::Healthy {
        return Err(
            media_match_health_message(health, &ffmpeg, &ffprobe).unwrap_or_else(|| {
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
    })
}

#[derive(Debug, Clone, Copy)]
struct MediaMatchDiscoveryLimits {
    max_depth: usize,
    max_nodes: usize,
}

fn collect_media_match_candidates(
    search_roots: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
) -> Result<Vec<PathBuf>, String> {
    discover_media_match_candidates(search_roots, cancel_flag).map(|discovery| discovery.candidates)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaMatchCandidateDiscovery {
    candidates: Vec<PathBuf>,
    scanned_roots: Vec<PathBuf>,
}

fn discover_media_match_candidates(
    search_roots: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
) -> Result<MediaMatchCandidateDiscovery, String> {
    discover_media_match_candidates_with_limits(
        search_roots,
        cancel_flag,
        MediaMatchDiscoveryLimits {
            max_depth: MEDIA_MATCH_DISCOVERY_MAX_DEPTH,
            max_nodes: MEDIA_MATCH_DISCOVERY_MAX_NODES,
        },
    )
}

#[cfg(test)]
fn collect_media_match_candidates_with_limits(
    search_roots: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
    limits: MediaMatchDiscoveryLimits,
) -> Result<Vec<PathBuf>, String> {
    discover_media_match_candidates_with_limits(search_roots, cancel_flag, limits)
        .map(|discovery| discovery.candidates)
}

fn discover_media_match_candidates_with_limits(
    search_roots: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
    limits: MediaMatchDiscoveryLimits,
) -> Result<MediaMatchCandidateDiscovery, String> {
    let mut files = Vec::new();
    // Configured roots are explicit trust boundaries. Follow a root link/junction for directory
    // validation and canonical cycle identity while preserving its configured spelling in
    // candidate paths. Descendant links remain rejected below.
    let mut stack = search_roots
        .iter()
        .cloned()
        .enumerate()
        .map(|(root_index, root)| (root, 0usize, true, root_index))
        .collect::<Vec<_>>();
    let mut visited_directories = HashSet::new();
    let mut root_scan_complete = vec![true; search_roots.len()];
    let mut root_was_traversed = vec![false; search_roots.len()];
    // Root work items are nodes too. Count them up front so an attacker cannot bypass the global
    // bound with a huge list of empty, duplicate, or nonexistent roots.
    let mut visited_nodes = search_roots.len();
    if visited_nodes > limits.max_nodes {
        return Err(format!(
            "Media Matching discovery exceeded its {}-node safety limit.",
            limits.max_nodes
        ));
    }
    while let Some((path, depth, is_explicit_root, root_index)) = stack.pop() {
        check_media_match_discovery_canceled(cancel_flag)?;
        let metadata = if is_explicit_root {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        };
        let Ok(metadata) = metadata else {
            root_scan_complete[root_index] = false;
            continue;
        };
        if !metadata.is_dir() {
            root_scan_complete[root_index] = false;
            continue;
        }
        let Ok(canonical_path) = fs::canonicalize(&path) else {
            root_scan_complete[root_index] = false;
            continue;
        };
        if !visited_directories.insert(media_match_directory_visit_key(&canonical_path)) {
            root_scan_complete[root_index] = false;
            continue;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            root_scan_complete[root_index] = false;
            continue;
        };
        if is_explicit_root {
            root_was_traversed[root_index] = true;
        }
        for entry in entries {
            check_media_match_discovery_canceled(cancel_flag)?;
            visited_nodes = visited_nodes.saturating_add(1);
            if visited_nodes > limits.max_nodes {
                return Err(format!(
                    "Media Matching discovery exceeded its {}-node safety limit.",
                    limits.max_nodes
                ));
            }
            let Ok(entry) = entry else {
                root_scan_complete[root_index] = false;
                continue;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                root_scan_complete[root_index] = false;
                continue;
            };
            if metadata_is_directory_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                if depth >= limits.max_depth {
                    return Err(format!(
                        "Media Matching discovery exceeded its {}-level depth safety limit at {}.",
                        limits.max_depth,
                        path.display()
                    ));
                }
                stack.push((path, depth + 1, false, root_index));
            } else if metadata.is_file() && media_match_candidate_extension(&path) {
                files.push(path);
            }
        }
    }
    files.sort_by(|left, right| {
        normalize_media_path(left)
            .cmp(&normalize_media_path(right))
            .then_with(|| left.cmp(right))
    });
    let scanned_roots = search_roots
        .iter()
        .enumerate()
        .filter(|(index, _)| root_was_traversed[*index] && root_scan_complete[*index])
        .map(|(_, root)| root.clone())
        .collect();
    Ok(MediaMatchCandidateDiscovery {
        candidates: files,
        scanned_roots,
    })
}

fn check_media_match_discovery_canceled(cancel_flag: Option<&AtomicBool>) -> Result<(), String> {
    if cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err("Media Matching discovery was canceled.".to_owned())
    } else {
        Ok(())
    }
}

fn metadata_is_directory_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn media_match_directory_visit_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        key.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    key
}

fn inventory_media_match_candidates(
    root: &Path,
    scanned_roots: &[PathBuf],
    candidates: &[PathBuf],
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), String> {
    let mut entries = Vec::with_capacity(candidates.len());
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
        entries.push(MediaIndexInventoryEntry::new(
            normalized_path,
            modified_unix_millis,
            size_bytes,
        ));
    }
    let normalized_roots = scanned_roots
        .iter()
        .map(normalize_media_path)
        .collect::<Vec<_>>();
    let seen_paths = candidates
        .iter()
        .map(normalize_media_path)
        .collect::<Vec<_>>();
    let result = open_media_match_index_session(root)?.refresh_inventory(
        &entries,
        &seen_paths,
        &normalized_roots,
        || cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)),
    );
    if result.is_err() && cancel_flag.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err("Media Matching inventory scan was canceled.".to_owned());
    }
    result.map(|_| ())
}

#[cfg(test)]
pub(super) fn rebuild_persisted_media_match_inventory_for_tests(
    root: &Path,
    search_roots: &[PathBuf],
) -> Result<(), String> {
    let discovery = discover_media_match_candidates(search_roots, None)?;
    inventory_media_match_candidates(root, &discovery.scanned_roots, &discovery.candidates, None)
}

fn media_match_path_is_under_root(normalized_path: &str, normalized_root: &str) -> bool {
    normalized_path == normalized_root
        || normalized_path
            .strip_prefix(normalized_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MediaMatchInventoryExactTarget {
    key: String,
    folded_key: String,
    file_name: String,
    folded_file_name: String,
    has_path_context: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaAliasMatchKind {
    ExactCase,
    FoldedCase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MediaMatchInventoryExactResolution {
    Resolved {
        path: String,
        match_kind: MediaAliasMatchKind,
    },
    Ambiguous {
        candidate_count: usize,
        match_kind: MediaAliasMatchKind,
    },
}

type MediaMatchInventoryCredibilityRank = (usize, usize, usize, usize, usize);

struct MediaMatchInventoryRankedCandidate {
    credibility_rank: MediaMatchInventoryCredibilityRank,
    path: String,
}

fn media_match_inventory_exact_target(target: &str) -> Option<MediaMatchInventoryExactTarget> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let target_path = Path::new(target);
    let file_name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let file_name = if cfg!(windows) {
        file_name.to_ascii_lowercase()
    } else {
        file_name.to_owned()
    };
    let key = normalize_media_path(target_path);
    let has_path_context = target.contains('/')
        || target.contains('\\')
        || target_path.is_absolute()
        || target_path.components().count() > 1;
    Some(MediaMatchInventoryExactTarget {
        folded_key: key.to_ascii_lowercase(),
        folded_file_name: file_name.to_ascii_lowercase(),
        key,
        file_name,
        has_path_context,
    })
}

fn media_match_inventory_path_target_rank(path: &str, target: &str) -> Option<usize> {
    if path == target {
        return Some(0);
    }
    path.strip_suffix(target)
        .filter(|prefix| prefix.ends_with('/'))
        .map(|_| 1)
}

fn media_match_inventory_exact_target_rank(
    normalized_path: &str,
    folded_path: &str,
    file_name: &str,
    folded_file_name: &str,
    target: &MediaMatchInventoryExactTarget,
) -> Option<(usize, usize)> {
    if target.has_path_context {
        if let Some(target_rank) =
            media_match_inventory_path_target_rank(normalized_path, &target.key)
        {
            return Some((0, target_rank));
        }
        return media_match_inventory_path_target_rank(folded_path, &target.folded_key)
            .map(|target_rank| (1, target_rank));
    }
    if file_name == target.file_name {
        return Some((0, 2));
    }
    (folded_file_name == target.folded_file_name).then_some((1, 2))
}

pub(super) fn media_match_inventory_exact_resolution_for_targets(
    root: &Path,
    search_roots: &[PathBuf],
    targets: &[String],
) -> Option<MediaMatchInventoryExactResolution> {
    let targets = targets
        .iter()
        .filter_map(|target| media_match_inventory_exact_target(target))
        .collect::<Vec<_>>();
    if targets.is_empty() || search_roots.is_empty() {
        return None;
    }

    let normalized_roots = search_roots
        .iter()
        .map(normalize_media_path)
        .collect::<Vec<_>>();
    let rows = open_media_match_index_session(root)
        .ok()?
        .inventory_paths()
        .ok()?;
    let mut best_match: Option<MediaMatchInventoryRankedCandidate> = None;
    let mut best_credibility_match_count = 0_usize;

    for row in rows {
        if !normalized_roots
            .iter()
            .any(|root| media_match_path_is_under_root(&row, root))
        {
            continue;
        }
        let path = Path::new(&row);
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()).map(|name| {
            if cfg!(windows) {
                name.to_ascii_lowercase()
            } else {
                name.to_owned()
            }
        }) else {
            continue;
        };
        let folded_path = row.to_ascii_lowercase();
        let folded_file_name = file_name.to_ascii_lowercase();
        let Some((alias_rank, case_rank, target_rank)) = targets
            .iter()
            .enumerate()
            .filter_map(|(alias_rank, target)| {
                media_match_inventory_exact_target_rank(
                    &row,
                    &folded_path,
                    &file_name,
                    &folded_file_name,
                    target,
                )
                .map(|(case_rank, target_rank)| (alias_rank, case_rank, target_rank))
            })
            .min()
        else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        let root_order = normalized_roots
            .iter()
            .position(|root| media_match_path_is_under_root(&row, root))
            .unwrap_or(usize::MAX);
        let depth = path.components().count();
        let credibility_rank = (alias_rank, case_rank, target_rank, root_order, depth);
        match best_match.as_ref() {
            None => {
                best_credibility_match_count = 1;
                best_match = Some(MediaMatchInventoryRankedCandidate {
                    credibility_rank,
                    path: row,
                });
            }
            Some(best) if credibility_rank < best.credibility_rank => {
                best_credibility_match_count = 1;
                best_match = Some(MediaMatchInventoryRankedCandidate {
                    credibility_rank,
                    path: row,
                });
            }
            Some(best) if credibility_rank == best.credibility_rank => {
                best_credibility_match_count += 1;
                if row < best.path {
                    best_match = Some(MediaMatchInventoryRankedCandidate {
                        credibility_rank,
                        path: row,
                    });
                }
            }
            Some(_) => {}
        }
    }

    let candidate = best_match?;
    let match_kind = if candidate.credibility_rank.1 == 0 {
        MediaAliasMatchKind::ExactCase
    } else {
        MediaAliasMatchKind::FoldedCase
    };
    if best_credibility_match_count > 1 {
        Some(MediaMatchInventoryExactResolution::Ambiguous {
            candidate_count: best_credibility_match_count,
            match_kind,
        })
    } else {
        Some(MediaMatchInventoryExactResolution::Resolved {
            path: candidate.path,
            match_kind,
        })
    }
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

fn select_remote_media_match_candidates(
    candidates: &[PathBuf],
    target_file_name: &str,
) -> MediaMatchRebuildCandidateSelection {
    let prefiltered = candidates.len() > MEDIA_MATCH_PREFILTER_THRESHOLD;
    let paths = if prefiltered {
        prefilter_media_match_candidates(candidates, target_file_name)
    } else {
        candidates.to_vec()
    };
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
    let mut tokens = Vec::new();
    let mut parent_names = path
        .parent()
        .into_iter()
        .flat_map(|parent| parent.ancestors())
        .filter_map(|parent| parent.file_name().and_then(|name| name.to_str()))
        .take(2)
        .collect::<Vec<_>>();
    parent_names.reverse();
    for parent_name in parent_names {
        tokens.extend(media_match_filename_component_tokens(parent_name));
    }
    tokens.extend(media_match_filename_component_tokens(
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    ));
    tokens
}

fn media_match_filename_component_tokens(component: &str) -> Vec<String> {
    let mut without_groups = String::new();
    let mut square_depth = 0u32;
    for character in component.chars() {
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

fn parallel_fresh_media_fingerprints<F>(
    paths: Vec<PathBuf>,
    tools: &MediaMatchToolPaths,
    extraction_settings: &MediaExtractionSettings,
    cancel_flag: Option<&AtomicBool>,
    mut progress: F,
) -> Result<
    (
        BTreeMap<String, MediaMatchParallelExtractionResult>,
        MediaMatchParallelExtractionStats,
    ),
    String,
>
where
    F: FnMut(usize, &Path),
{
    if paths.is_empty() {
        return Ok(Default::default());
    }
    let worker_count = media_match_extraction_worker_count(extraction_settings).min(paths.len());
    let jobs = Arc::new(Mutex::new(
        paths
            .iter()
            .cloned()
            .map(|path| (path, Instant::now()))
            .collect::<VecDeque<_>>(),
    ));
    let (tx, rx) = mpsc::channel::<MediaMatchParallelExtractionOutput>();
    let tools = tools.clone();
    let extraction_settings = extraction_settings.clone();
    let no_cancel = AtomicBool::new(false);
    let cancel_flag = cancel_flag.unwrap_or(&no_cancel);
    let mut outputs = Vec::new();

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let jobs = Arc::clone(&jobs);
            let tx = tx.clone();
            let tools = tools.clone();
            let extraction_settings = extraction_settings.clone();
            scope.spawn(move || {
                loop {
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some((path, queued_at)) =
                        jobs.lock().ok().and_then(|mut jobs| jobs.pop_front())
                    else {
                        break;
                    };
                    let queue_wait_millis = queued_at.elapsed().as_millis();
                    let worker_started_at = Instant::now();
                    let result = fingerprint_media_file_cancellable_with_report(
                        &path,
                        &tools,
                        &extraction_settings,
                        cancel_flag,
                    )
                    .map(|fingerprint| (fingerprint.record, fingerprint.report));
                    let output = MediaMatchParallelExtractionOutput {
                        path,
                        queue_wait_millis,
                        worker_wall_millis: worker_started_at.elapsed().as_millis(),
                        result,
                    };
                    if tx.send(output).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for output in rx {
            progress(outputs.len() + 1, &output.path);
            outputs.push(output);
        }
    });

    let cancel_requested = cancel_flag.load(Ordering::Relaxed);
    let mut stats = MediaMatchParallelExtractionStats {
        background_index_worker_count: worker_count,
        sampled_fast_worker_count: worker_count,
        ..MediaMatchParallelExtractionStats::default()
    };
    let mut results = BTreeMap::new();
    for output in outputs {
        stats.extraction_queue_wait_millis = stats
            .extraction_queue_wait_millis
            .saturating_add(output.queue_wait_millis);
        stats.extraction_worker_wall_millis = stats
            .extraction_worker_wall_millis
            .saturating_add(output.worker_wall_millis);
        if output.result.is_ok() {
            stats.files_indexed += 1;
        }
        if matches!(&output.result, Err(MediaFingerprintError::Cancelled { .. })) {
            stats.cancelled_file_count += 1;
        }
        results.insert(normalize_media_path(&output.path), output.result);
    }
    if cancel_requested {
        stats.cancelled_file_count = stats
            .cancelled_file_count
            .saturating_add(paths.len().saturating_sub(results.len()));
    }
    Ok((results, stats))
}

fn media_match_extraction_worker_count(extraction_settings: &MediaExtractionSettings) -> usize {
    let cores = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    let _ = extraction_settings;
    cores.clamp(1, 8)
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

#[cfg(test)]
fn media_match_anchor_candidate_paths(
    root: &Path,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<Vec<String>, String> {
    media_match_v3_anchor_candidate_paths(root, normalized_current_path, extraction_settings)
}

#[cfg(test)]
fn media_match_v3_anchor_candidate_paths(
    root: &Path,
    normalized_current_path: &str,
    extraction_settings: &MediaExtractionSettings,
) -> Result<Vec<String>, String> {
    MediaIndexService::new(managed_media_match_index_dir(root))
        .open()?
        .anchor_candidate_paths(normalized_current_path, extraction_settings)
        .map(|(paths, _stats)| paths)
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
    let (anchor_candidates, retrieval_stats) =
        MediaIndexService::new(managed_media_match_index_dir(root))
            .open()
            .and_then(|session| {
                session.anchor_candidate_paths(&normalized_current_path, extraction_settings)
            })
            .map(|(paths, stats)| (paths, Some(stats)))
            .unwrap_or_default();
    let retrieval_suffix = retrieval_stats
        .as_ref()
        .map(format_media_match_v3_retrieval_stats)
        .unwrap_or_default();
    // TODO(media-match): the runtime owner has playback position as
    // `sorotte_client_core::session::SessionState::local_position`; thread that value into
    // `summarize_current_media_match` and append `format_media_match_position_mapping_diagnostic`
    // to debug evidence only. Do not infer across edit gaps or change readiness/autoplay/seek
    // behavior when this diagnostic is wired.
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
            Some(format!(
                "current file is indexed exactly; no alternate indexed candidates{retrieval_suffix}"
            )),
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
                "current file is indexed exactly | nearest_other {}{retrieval_suffix}",
                format_media_match_evidence_summary(&best.decision),
            )),
        );
    }
    let tier = media_match_tier_label(best.decision.tier);
    (
        Some(format!("{tier}: {}", best.decision.explanation)),
        Some(format_media_match_nearest_candidate(&best)),
        Some(format!(
            "{}{retrieval_suffix}",
            format_media_match_evidence_summary(&best.decision)
        )),
    )
}

pub(super) fn media_match_cached_probable_candidate_for_remote_signature(
    root: &Path,
    search_roots: &[PathBuf],
    candidates: Option<&[PathBuf]>,
    target_file_name: &str,
    media_match_signature: &sorotte_media_match::MediaMatchWireSignature,
    settings: &MediaMatchSettings,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchRemoteCandidateMatch> {
    let candidates = match candidates {
        Some(candidates) => candidates.to_vec(),
        None => collect_media_match_candidates(search_roots, None).ok()?,
    };
    let selected = select_remote_media_match_candidates(&candidates, target_file_name);
    let cache = load_media_match_cache_for_settings(root, extraction_settings)?;
    best_remote_candidate_match(
        &selected.paths,
        &cache,
        media_match_signature,
        settings,
        extraction_settings,
    )
    .filter(|candidate| media_match_tier_is_probable_or_better(candidate.decision.tier))
}

fn best_remote_candidate_match(
    candidates: &[PathBuf],
    cache: &MediaMatchCache,
    signature: &sorotte_media_match::MediaMatchWireSignature,
    settings: &MediaMatchSettings,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchRemoteCandidateMatch> {
    candidates
        .iter()
        .filter(|path| media_match_cache_has_valid_record(cache, path, extraction_settings))
        .filter_map(|path| {
            let normalized_path = normalize_media_path(path);
            let record = cache.records.get(&normalized_path)?;
            let decision = sorotte_media_match::decide_media_match_against_wire_signature(
                record, signature, settings,
            );
            Some(MediaMatchRemoteCandidateMatch {
                path: record.identity.normalized_path.clone(),
                decision,
            })
        })
        .max_by(|left, right| media_match_decision_cmp(&left.decision, &right.decision))
}

fn media_match_tier_is_strong_or_exact(tier: MediaMatchTier) -> bool {
    matches!(tier, MediaMatchTier::Exact | MediaMatchTier::Strong)
}

fn media_match_tier_is_probable_or_better(tier: MediaMatchTier) -> bool {
    matches!(
        tier,
        MediaMatchTier::Exact | MediaMatchTier::Strong | MediaMatchTier::Probable
    )
}

fn media_match_decision_is_better(
    candidate: &MediaMatchDecision,
    current: &MediaMatchDecision,
) -> bool {
    media_match_decision_cmp(candidate, current).is_gt()
}

fn media_match_decision_cmp(
    left: &MediaMatchDecision,
    right: &MediaMatchDecision,
) -> std::cmp::Ordering {
    media_match_tier_score(left.tier)
        .cmp(&media_match_tier_score(right.tier))
        .then_with(|| {
            left.evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.aligned_pairs)
                .unwrap_or_default()
                .cmp(
                    &right
                        .evidence
                        .alignment
                        .as_ref()
                        .map(|alignment| alignment.aligned_pairs)
                        .unwrap_or_default(),
                )
        })
        .then_with(|| {
            left.evidence
                .alignment
                .as_ref()
                .map(|alignment| alignment.aligned_span_seconds as u64)
                .unwrap_or_default()
                .cmp(
                    &right
                        .evidence
                        .alignment
                        .as_ref()
                        .map(|alignment| alignment.aligned_span_seconds as u64)
                        .unwrap_or_default(),
                )
        })
}

fn media_match_tier_score(tier: MediaMatchTier) -> u8 {
    match tier {
        MediaMatchTier::Exact => 5,
        MediaMatchTier::Strong => 4,
        MediaMatchTier::Probable => 3,
        MediaMatchTier::Weak => 2,
        MediaMatchTier::Reject => 1,
        MediaMatchTier::Unknown => 0,
    }
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
            "alignment offset={:.1}s scale={}ppm drift={:.4} span={:.1}s pairs={} audio={} margin={:.2}",
            alignment.offset_seconds,
            alignment.scale_ppm,
            alignment.drift_ratio,
            alignment.aligned_span_seconds,
            alignment.aligned_pairs,
            alignment.aligned_audio_anchors,
            alignment.second_best_offset_margin
        ));
    }
    if let Some(audio) = decision.evidence.audio.as_ref() {
        parts.push(format!(
            "audio similarity={:.2} shared={:.2} duration_delta={}",
            audio.similarity,
            audio.shared_anchor_ratio,
            format_optional_seconds(audio.duration_delta_seconds)
        ));
    }
    if let Some(map) = decision.evidence.timeline_map_v3.as_ref() {
        parts.push(format!(
            "v3 class={:?} segments={} span={:.1}s largest_gap={:.1}s edge_only={} best_segment={} second_segment={}",
            map.global_class,
            map.segments.len(),
            f64::from(map.total_aligned_span_ms) / 1000.0,
            f64::from(map.largest_gap_ms) / 1000.0,
            map.edge_only,
            map.best_segment_score,
            map.second_best_segment_score
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

// TODO(media-match): wire `sorotte_client_core::session::SessionState::local_position`
// into `summarize_current_media_match` and use this formatting path for debug evidence only.
// The current summary inputs have the best V3 decision but not the player's current timestamp.
#[cfg(test)]
fn format_media_match_position_mapping_diagnostic(
    decision: &MediaMatchDecision,
    current_position_ms: u32,
) -> Option<String> {
    let map = decision.evidence.timeline_map_v3.as_ref()?;
    let mapped = map_query_position_to_candidate_ms(map, current_position_ms)?;
    Some(format!(
        "mapped_position candidate={:.1}s class={:?} segment={} confidence={:.2} local_offset={}ms scale={}ppm",
        f64::from(mapped.mapped_ms) / 1000.0,
        mapped.class_at_position,
        mapped.segment_index,
        mapped.confidence,
        mapped.local_offset_ms,
        mapped.scale_ppm
    ))
}

fn format_media_match_v3_retrieval_stats(stats: &MediaMatchV3RetrievalStats) -> String {
    format!(
        " | retrieval buckets={}/{} skipped_common={} hits={} candidates={} elapsed={}ms",
        stats
            .query_buckets_total
            .saturating_sub(stats.query_buckets_skipped_common),
        stats.query_buckets_total,
        stats.query_buckets_skipped_common,
        stats.raw_hit_rows_processed,
        stats.candidates_scored,
        stats.retrieval_elapsed_ms
    )
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
    let output =
        probe_executable_output_with_timeout(path, args, MEDIA_MATCH_VERSION_PROBE_TIMEOUT)?;
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

fn probe_executable_output_with_timeout(
    path: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    // Version probes use bounded `-version` output from ffmpeg/ffprobe. Keeping
    // stdout/stderr piped is acceptable here because the process is also
    // timeout-protected; long-running media extraction uses the streaming
    // runner that drains pipes concurrently.
    let mut child = hidden_media_match_command(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to run '{} {}': {error}",
                path.display(),
                args.join(" ")
            )
        })?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child.wait_with_output().map_err(|error| {
                    format!(
                        "failed collecting output from '{} {}': {error}",
                        path.display(),
                        args.join(" ")
                    )
                });
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "timed out after {:.1}s running '{} {}'",
                    timeout.as_secs_f64(),
                    path.display(),
                    args.join(" ")
                ));
            }
            Ok(None) => std::thread::sleep(MEDIA_MATCH_VERSION_PROBE_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "failed waiting for '{} {}': {error}",
                    path.display(),
                    args.join(" ")
                ));
            }
        }
    }
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

fn open_media_match_index_session(root: &Path) -> Result<MediaIndexSession, String> {
    MediaIndexService::new(managed_media_match_index_dir(root)).open()
}

pub(super) fn load_media_match_cache_for_settings(
    root: &Path,
    extraction_settings: &MediaExtractionSettings,
) -> Option<MediaMatchCache> {
    let session = MediaIndexService::new(managed_media_match_index_dir(root))
        .open()
        .ok()?;
    session
        .load_cache(extraction_settings)
        .ok()
        .filter(|cache| !cache.records.is_empty())
}

pub(super) fn media_match_wire_value_for_path(
    root: &Path,
    current_player_path: &str,
) -> Option<serde_json::Value> {
    let record = media_match_record_for_path(
        root,
        current_player_path,
        &MediaExtractionSettings::sampled_fast_audio_index_v3(),
    )?;
    media_match_wire_value_from_records(std::slice::from_ref(&record))
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
    let session = MediaIndexService::new(managed_media_match_index_dir(root))
        .open()
        .ok()?;
    session
        .load_record(
            &normalized_path,
            extraction_settings,
            modified_unix_millis,
            size_bytes,
        )
        .ok()
        .flatten()
}

#[cfg(test)]
fn save_media_match_cache(root: &Path, cache: &MediaMatchCache) -> Result<(), String> {
    let session = open_media_match_index_session(root)?;
    save_media_match_cache_to_index(&session, cache)
}

#[cfg(test)]
pub(in crate::app) fn save_media_match_cache_for_test(
    root: &Path,
    cache: &MediaMatchCache,
) -> Result<(), String> {
    save_media_match_cache(root, cache)
}

#[cfg(test)]
fn save_media_match_cache_to_index(
    session: &MediaIndexSession,
    cache: &MediaMatchCache,
) -> Result<(), String> {
    for record in cache.records.values() {
        session.save_record(record, None)?;
    }
    Ok(())
}

#[cfg(test)]
fn save_media_match_record_to_index(
    session: &MediaIndexSession,
    record: &MediaFingerprintRecord,
) -> Result<(), String> {
    session.save_record(record, None)
}

fn refresh_media_match_v3_anchor_stats_for_settings(
    session: &MediaIndexSession,
    extraction_settings: &MediaExtractionSettings,
    instrumentation: &mut MediaMatchRebuildInstrumentation,
) -> Result<(), String> {
    let settings_hash = media_extraction_settings_hash(extraction_settings);
    let now = current_unix_millis() as i64;
    let start = Instant::now();
    session.refresh_anchor_stats(&settings_hash, now)?;
    instrumentation.add_stats_refresh(start.elapsed().as_millis());
    Ok(())
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
            audio_anchors: Vec::new(),
            audio_error: None,
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
            audio_anchors: Vec::new(),
            audio_error: None,
        }
    }

    fn enabled_media_match_settings() -> MediaMatchSettings {
        MediaMatchSettings {
            fingerprinting_enabled: true,
            ..MediaMatchSettings::default()
        }
    }

    #[test]
    fn media_match_discovery_honors_cancellation_before_traversal() {
        let root = unique_media_match_test_root("discovery-cancel");
        std::fs::write(root.join("episode.mkv"), b"media").unwrap();
        let canceled = AtomicBool::new(true);

        let error = collect_media_match_candidates(&[root], Some(&canceled))
            .expect_err("a canceled discovery must stop before scanning");

        assert!(error.contains("discovery was canceled"));
    }

    #[test]
    fn media_match_discovery_deduplicates_canonical_directories_and_bounds_nodes() {
        let root = unique_media_match_test_root("discovery-visited");
        std::fs::write(root.join("episode.mkv"), b"media").unwrap();
        let duplicate_root = root.join(".");

        let candidates = collect_media_match_candidates(&[root.clone(), duplicate_root], None)
            .expect("duplicate roots should remain bounded");
        assert_eq!(candidates, vec![root.join("episode.mkv")]);

        let error = collect_media_match_candidates_with_limits(
            &[root],
            None,
            MediaMatchDiscoveryLimits {
                max_depth: 64,
                max_nodes: 1,
            },
        )
        .expect_err("directory entries after the root must count toward the node bound");
        assert!(error.contains("node safety limit"));

        let duplicate_roots = [
            unique_media_match_test_root("discovery-root-bound-a"),
            unique_media_match_test_root("discovery-root-bound-b"),
        ];
        let error = collect_media_match_candidates_with_limits(
            &duplicate_roots,
            None,
            MediaMatchDiscoveryLimits {
                max_depth: 64,
                max_nodes: 1,
            },
        )
        .expect_err("root work items themselves must count toward the node bound");
        assert!(error.contains("node safety limit"));
    }

    #[test]
    fn media_match_discovery_enforces_depth_bound() {
        let root = unique_media_match_test_root("discovery-depth");
        let nested = root.join("one").join("two");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("episode.mkv"), b"media").unwrap();

        let error = collect_media_match_candidates_with_limits(
            &[root],
            None,
            MediaMatchDiscoveryLimits {
                max_depth: 1,
                max_nodes: 100,
            },
        )
        .expect_err("the configured depth bound must stop traversal");
        assert!(error.contains("depth safety limit"));
    }

    #[test]
    fn media_match_discovery_skips_directory_links_and_cycles() {
        let root = unique_media_match_test_root("discovery-link-cycle");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("episode.mkv"), b"media").unwrap();
        let link = nested.join("back-to-root");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&root, &link).is_err() {
            // Windows CI hosts can disable unprivileged symlink creation. The reparse-point
            // branch is still exercised when Developer Mode or the test privilege is present.
            return;
        }

        let candidates = collect_media_match_candidates(&[root], None)
            .expect("a directory-link cycle must be skipped");
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].ends_with(Path::new("nested").join("episode.mkv")));
    }

    #[test]
    fn media_match_discovery_accepts_an_explicit_directory_symlink_root() {
        let container = unique_media_match_test_root("discovery-explicit-link-root");
        let target = container.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("episode.mkv"), b"media").unwrap();
        let link = container.join("configured-root");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
            return;
        }

        let candidates = collect_media_match_candidates(std::slice::from_ref(&link), None)
            .expect("an explicitly configured directory link should resolve to its target");
        assert_eq!(candidates, vec![link.join("episode.mkv")]);
    }

    #[cfg(windows)]
    #[test]
    fn media_match_discovery_accepts_an_explicit_windows_junction_root() {
        let container = unique_media_match_test_root("discovery-explicit-junction-root");
        let target = container.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("episode.mkv"), b"media").unwrap();
        let junction = container.join("configured-root");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .status()
            .expect("junction command should start");
        assert!(status.success(), "test junction should be created");

        let candidates = collect_media_match_candidates(std::slice::from_ref(&junction), None)
            .expect("an explicitly configured junction should resolve to its target");
        assert_eq!(candidates, vec![junction.join("episode.mkv")]);
    }

    fn healthy_tool_probe(name: &str) -> MediaMatchToolProbe {
        MediaMatchToolProbe {
            path: Some(PathBuf::from(name)),
            error: None,
            status: format!("{name} ok"),
        }
    }

    fn seed_strong_anchor_fixture(record: &mut MediaFingerprintRecord) {
        record.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        record.duration_seconds = Some(900.0);
        record.audio_anchors = (0u32..24)
            .map(|index| AudioAnchor {
                bucket: 100 + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
    }

    fn media_match_record_updated_unix_millis(root: &Path, record: &MediaFingerprintRecord) -> i64 {
        open_media_match_index_session(root)
            .expect("media index should open")
            .record_updated_unix_millis(
                &record.identity.normalized_path,
                &record.extraction_settings,
            )
            .expect("record timestamp query should run")
            .expect("record timestamp should be readable")
    }

    fn v3_audio_bucket_document_frequency(
        session: &MediaIndexSession,
        settings_hash: &[u8; 32],
        bucket: u32,
    ) -> Option<i64> {
        session
            .audio_bucket_document_frequency(settings_hash, bucket)
            .expect("anchor stats frequency query should run")
    }

    fn v3_record_with_audio_anchors(
        path: &str,
        anchors: &[(u32, u32, u16)],
    ) -> MediaFingerprintRecord {
        let mut record = fake_media_match_record(path);
        record.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        record.duration_seconds = Some(1_200.0);
        record.audio_anchors = anchors
            .iter()
            .map(|(bucket, t_ms, weight)| AudioAnchor {
                bucket: *bucket,
                t_ms: *t_ms,
                weight: *weight,
            })
            .collect();
        record
    }

    #[test]
    fn v3_tool_readiness_requires_ffmpeg_and_ffprobe() {
        let ffmpeg = healthy_tool_probe("ffmpeg");
        let ffprobe = healthy_tool_probe("ffprobe");

        assert_eq!(
            media_match_health_for_settings(
                &ffmpeg,
                &ffprobe,
                &MediaExtractionSettings::sampled_fast_audio_index_v3()
            ),
            GuiMediaMatchToolHealth::Healthy
        );
        assert_eq!(
            media_match_health_for_settings(
                &ffmpeg,
                &ffprobe,
                &MediaExtractionSettings::sampled_fast_audio_index_v3()
            ),
            GuiMediaMatchToolHealth::Healthy
        );
    }

    #[test]
    fn exact_inventory_path_aliases_remain_ascii_case_insensitive() {
        let root = unique_media_match_test_root("exact-inventory-case-insensitive-alias");
        let media_root = root.join("MixedCaseLibrary");
        let media_path = media_root.join("Show.S01E01.mkv");
        std::fs::create_dir_all(&media_root).expect("mixed-case media root should be created");
        std::fs::write(&media_path, b"fixture").expect("mixed-case media file should be written");
        inventory_media_match_candidates(
            &root,
            std::slice::from_ref(&media_root),
            std::slice::from_ref(&media_path),
            None,
        )
        .expect("mixed-case media path should be inventoried");

        let resolution = media_match_inventory_exact_resolution_for_targets(
            &root,
            std::slice::from_ref(&media_root),
            &["mixedcaselibrary/SHOW.S01E01.MKV".to_owned()],
        );
        let expected = normalize_media_path(&media_path);
        let expected_match_kind = if cfg!(windows) {
            MediaAliasMatchKind::ExactCase
        } else {
            MediaAliasMatchKind::FoldedCase
        };

        assert_eq!(
            resolution,
            Some(MediaMatchInventoryExactResolution::Resolved {
                path: expected.clone(),
                match_kind: expected_match_kind,
            })
        );
        let Some(MediaMatchInventoryExactResolution::Resolved { path: resolved, .. }) = resolution
        else {
            panic!("differently cased path alias should resolve");
        };
        assert!(Path::new(&resolved).is_file());
        std::fs::remove_dir_all(root).expect("temporary media root should be removable");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exact_inventory_prefers_exact_case_and_reports_folded_case_collisions() {
        let root = unique_media_match_test_root("exact-inventory-case-collision");
        let media_root = root.join("library");
        let upper_path = media_root.join("Pilot.mkv");
        let lower_path = media_root.join("pilot.mkv");
        std::fs::create_dir_all(&media_root).expect("case-collision media root should be created");
        std::fs::write(&upper_path, b"upper").expect("uppercase media fixture should be written");
        std::fs::write(&lower_path, b"lower").expect("lowercase media fixture should be written");
        inventory_media_match_candidates(
            &root,
            std::slice::from_ref(&media_root),
            &[upper_path, lower_path.clone()],
            None,
        )
        .expect("case-distinct paths should be inventoried");

        assert_eq!(
            media_match_inventory_exact_resolution_for_targets(
                &root,
                std::slice::from_ref(&media_root),
                &["pilot.mkv".to_owned()],
            ),
            Some(MediaMatchInventoryExactResolution::Resolved {
                path: normalize_media_path(&lower_path),
                match_kind: MediaAliasMatchKind::ExactCase,
            }),
            "an exact-case alias must outrank a case-folded alias"
        );
        assert_eq!(
            media_match_inventory_exact_resolution_for_targets(
                &root,
                std::slice::from_ref(&media_root),
                &["PILOT.MKV".to_owned()],
            ),
            Some(MediaMatchInventoryExactResolution::Ambiguous {
                candidate_count: 2,
                match_kind: MediaAliasMatchKind::FoldedCase,
            }),
            "equally ranked case-folded aliases must not resolve lexically"
        );

        std::fs::remove_dir_all(root).expect("temporary media root should be removable");
    }

    #[test]
    fn runtime_snapshot_defaults_to_v3_tools_only() {
        let settings = enabled_media_match_settings();
        let snapshot = media_match_runtime_snapshot_from_probes(
            None,
            &settings,
            healthy_tool_probe("ffmpeg"),
            healthy_tool_probe("ffprobe"),
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );

        assert_eq!(snapshot.health, GuiMediaMatchToolHealth::Healthy);
        assert_eq!(snapshot.message, None);
    }

    #[test]
    fn v3_install_readiness_text_names_required_tools() {
        let success = media_match_install_success_message();
        assert!(success.contains("Media Matching V3"), "{success}");
        assert!(success.contains("ffmpeg and ffprobe"), "{success}");
        assert!(
            media_match_health_message(
                GuiMediaMatchToolHealth::MissingFfmpeg,
                &healthy_tool_probe("ffmpeg"),
                &healthy_tool_probe("ffprobe"),
            )
            .expect("missing ffmpeg message should exist")
            .contains("ffmpeg")
        );
    }

    #[test]
    fn media_match_v3_anchor_stats_refreshes_explicitly_after_saves() {
        let root = unique_media_match_test_root("v3-stats-refresh");
        let session = open_media_match_index_session(&root).expect("media index should open");
        let mut record = fake_media_match_record("episode.mkv");
        seed_strong_anchor_fixture(&mut record);
        save_media_match_record_to_index(&session, &record)
            .expect("V3 record should save without refreshing stats");
        let settings_hash = media_extraction_settings_hash(&record.extraction_settings);
        assert!(
            session
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "checkpointed V3 saves must mark active settings stats dirty"
        );

        let stats_before = session
            .positive_anchor_bucket_count()
            .expect("stats count should load");
        assert_eq!(
            stats_before, 0,
            "per-record V3 saves should not refresh anchor stats"
        );

        let mut instrumentation = MediaMatchRebuildInstrumentation::default();
        refresh_media_match_v3_anchor_stats_for_settings(
            &session,
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            &mut instrumentation,
        )
        .expect("batch stats refresh should succeed");
        let stats_after = session
            .positive_anchor_bucket_count()
            .expect("stats count should load");

        assert!(stats_after > 0);
        assert_eq!(instrumentation.stats_refreshes, 1);
        assert!(
            !session
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "explicit per-settings refresh should clear the scoped dirty marker"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dirty_stats_after_checkpoint_refreshes_on_candidate_lookup() {
        let root = unique_media_match_test_root("v3-dirty-checkpoint");
        let session = open_media_match_index_session(&root).expect("media index should open");
        let query = v3_record_with_audio_anchors(
            "query.mkv",
            &[(10, 100_000, 4), (11, 160_000, 4), (12, 220_000, 4)],
        );
        let candidate = v3_record_with_audio_anchors(
            "candidate.mkv",
            &[(10, 105_000, 4), (11, 165_000, 4), (12, 225_000, 4)],
        );
        save_media_match_record_to_index(&session, &query).expect("query should save");
        save_media_match_record_to_index(&session, &candidate).expect("candidate should save");
        let settings_hash = media_extraction_settings_hash(&query.extraction_settings);
        assert!(
            session
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "a checkpoint before batch refresh should leave stats dirty"
        );

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("candidate lookup should refresh dirty stats");

        assert!(candidates.contains(&candidate.identity.normalized_path));
        let refreshed = open_media_match_index_session(&root).expect("media index should reopen");
        assert!(
            !refreshed
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "candidate lookup should refresh stats and clear the active dirty marker"
        );
        assert_eq!(
            v3_audio_bucket_document_frequency(&refreshed, &settings_hash, 10),
            Some(2)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn offset_aware_retrieval_prefers_dominant_offset_cluster() {
        let root = unique_media_match_test_root("v3-offset-retrieval");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let query_anchors = (0..8)
            .map(|index| (100 + index, 120_000 + (index * 60_000), 4))
            .collect::<Vec<_>>();
        let query = v3_record_with_audio_anchors("query.mkv", &query_anchors);
        let clustered = v3_record_with_audio_anchors(
            "clustered.mkv",
            &query_anchors[..6]
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        let scattered = v3_record_with_audio_anchors(
            "scattered.mkv",
            &query_anchors
                .iter()
                .enumerate()
                .map(|(index, (bucket, t_ms, weight))| {
                    (*bucket, *t_ms + 5_000 + (index as u32 * 7_000), *weight)
                })
                .collect::<Vec<_>>(),
        );
        for record in [&query, &scattered, &clustered] {
            save_media_match_record_to_index(&connection, record).expect("record should save");
        }

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("candidate lookup should run");

        assert_eq!(
            candidates.first(),
            Some(&clustered.identity.normalized_path)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn offset_aware_retrieval_downweights_high_document_frequency_buckets() {
        let root = unique_media_match_test_root("v3-offset-df");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let common = (0..6)
            .map(|index| (500 + index, 120_000 + (index * 60_000), 4))
            .collect::<Vec<_>>();
        let rare = (0..3)
            .map(|index| (900 + index, 540_000 + (index * 60_000), 4))
            .collect::<Vec<_>>();
        let mut query_anchors = common.clone();
        query_anchors.extend(rare.clone());
        let query = v3_record_with_audio_anchors("query.mkv", &query_anchors);
        let common_candidate = v3_record_with_audio_anchors(
            "common-candidate.mkv",
            &common
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        let rare_candidate = v3_record_with_audio_anchors(
            "rare-candidate.mkv",
            &rare
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        save_media_match_record_to_index(&connection, &query).expect("query should save");
        save_media_match_record_to_index(&connection, &common_candidate)
            .expect("common candidate should save");
        save_media_match_record_to_index(&connection, &rare_candidate)
            .expect("rare candidate should save");
        for dummy_index in 0..260 {
            let dummy = v3_record_with_audio_anchors(
                &format!("dummy-{dummy_index}.mkv"),
                &common
                    .iter()
                    .enumerate()
                    .map(|(index, (bucket, t_ms, weight))| {
                        (*bucket, *t_ms + 30_000 + (index as u32 * 9_000), *weight)
                    })
                    .collect::<Vec<_>>(),
            );
            save_media_match_record_to_index(&connection, &dummy).expect("dummy should save");
        }

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("candidate lookup should run");

        assert_eq!(
            candidates.first(),
            Some(&rare_candidate.identity.normalized_path),
            "rare offset-cluster evidence should outrank higher-count common buckets"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn retrieval_common_bucket_cap_skips_overly_common_buckets() {
        let root = unique_media_match_test_root("v3-common-bucket-cap");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let query =
            v3_record_with_audio_anchors("query.mkv", &[(777, 120_000, 4), (999, 420_000, 4)]);
        let common_candidate =
            v3_record_with_audio_anchors("common-candidate.mkv", &[(777, 125_000, 4)]);
        let rare_candidate =
            v3_record_with_audio_anchors("rare-candidate.mkv", &[(999, 425_000, 4)]);
        for record in [&query, &common_candidate, &rare_candidate] {
            save_media_match_record_to_index(&connection, record).expect("record should save");
        }
        for dummy_index in 0..260 {
            let dummy = v3_record_with_audio_anchors(
                &format!("common-dummy-{dummy_index}.mkv"),
                &[(777, 180_000 + dummy_index, 1)],
            );
            save_media_match_record_to_index(&connection, &dummy).expect("dummy should save");
        }

        let (candidates, stats) = connection
            .anchor_candidate_paths(&query.identity.normalized_path, &query.extraction_settings)
            .expect("candidate lookup should run");

        assert!(stats.query_buckets_total >= 2, "{stats:?}");
        assert!(stats.query_buckets_skipped_common >= 1, "{stats:?}");
        assert!(stats.raw_hit_rows_processed >= 1, "{stats:?}");
        assert!(stats.candidates_scored >= 1, "{stats:?}");
        assert_eq!(
            candidates.first(),
            Some(&rare_candidate.identity.normalized_path),
            "rare buckets should survive when overly common buckets are skipped"
        );
        assert!(
            !candidates.contains(&common_candidate.identity.normalized_path),
            "candidate with only an over-common bucket should be skipped"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn v3_runtime_and_diagnostic_retrieval_share_index_results() {
        let root = unique_media_match_test_root("v3-shared-retrieval");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let query = v3_record_with_audio_anchors(
            "query.mkv",
            &[
                (1_000, 120_000, 4),
                (1_001, 180_000, 4),
                (1_002, 240_000, 4),
            ],
        );
        let candidate = v3_record_with_audio_anchors(
            "candidate.mkv",
            &[
                (1_000, 126_000, 4),
                (1_001, 186_000, 4),
                (1_002, 246_000, 4),
            ],
        );
        let scattered = v3_record_with_audio_anchors(
            "scattered.mkv",
            &[
                (1_000, 300_000, 4),
                (1_001, 900_000, 4),
                (1_002, 450_000, 4),
            ],
        );
        for record in [&query, &candidate, &scattered] {
            save_media_match_record_to_index(&connection, record).expect("record should save");
        }

        let runtime_candidates = media_match_v3_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("runtime candidate lookup should run");
        let (diagnostic_candidates, stats) = connection
            .anchor_candidate_paths(&query.identity.normalized_path, &query.extraction_settings)
            .expect("diagnostic candidate lookup should run");

        assert_eq!(runtime_candidates, diagnostic_candidates);
        assert_eq!(
            diagnostic_candidates.first(),
            Some(&candidate.identity.normalized_path)
        );
        assert!(stats.raw_hit_rows_processed > 0, "{stats:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saved_v3_record_loads_through_shared_and_runtime_paths() {
        let root = unique_media_match_test_root("v3-shared-load");
        let media_path = root.join("media.mkv");
        std::fs::write(&media_path, b"same media bytes").expect("media file should be written");
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let mut record = fake_media_match_record_for_file(&media_path, extraction_settings.clone());
        record.audio_anchors = vec![
            AudioAnchor {
                bucket: 700,
                t_ms: 10_000,
                weight: 4,
            },
            AudioAnchor {
                bucket: 701,
                t_ms: 40_000,
                weight: 4,
            },
        ];
        let connection = open_media_match_index_session(&root).expect("media index should open");
        save_media_match_record_to_index(&connection, &record).expect("record should save");

        let shared = connection
            .load_record(
                &record.identity.normalized_path,
                &extraction_settings,
                record.identity.modified_unix_millis,
                record.identity.size_bytes,
            )
            .expect("shared loader should run")
            .expect("shared loader should return saved record");
        let runtime = media_match_record_for_path(
            &root,
            media_path.to_str().expect("test path should be UTF-8"),
            &extraction_settings,
        )
        .expect("runtime loader should return saved record");

        assert_eq!(shared.identity, runtime.identity);
        assert_eq!(shared.audio_anchors, runtime.audio_anchors);
        assert_eq!(shared.extraction_settings, runtime.extraction_settings);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mapped_position_diagnostic_formats_known_v3_segment() {
        let decision = MediaMatchDecision {
            tier: MediaMatchTier::Strong,
            evidence: sorotte_media_match::MediaMatchEvidence {
                timeline_map_v3: Some(sorotte_media_match::MediaTimelineMapV3 {
                    global_class: sorotte_media_match::MatchClassV3::SameCutStrong,
                    current_position_class: sorotte_media_match::MatchClassV3::SameCutStrong,
                    segments: vec![sorotte_media_match::AlignedSegmentV3 {
                        query_start_ms: 10_000,
                        query_end_ms: 40_000,
                        candidate_start_ms: 15_000,
                        candidate_end_ms: 45_000,
                        scale_ppm: 1_000_000,
                        audio_pairs: 8,
                        weighted_score: 32,
                        residual_ms: 0.0,
                        audio_score: 1.0,
                        confidence: 0.9,
                    }],
                    total_aligned_span_ms: 30_000,
                    largest_gap_ms: 0,
                    edge_only: false,
                    best_segment_score: 32,
                    second_best_segment_score: 0,
                }),
                ..sorotte_media_match::MediaMatchEvidence::default()
            },
            explanation: "same cut".to_owned(),
        };

        let diagnostic = format_media_match_position_mapping_diagnostic(&decision, 20_000)
            .expect("position inside segment should map");

        assert!(diagnostic.contains("candidate=25.0s"), "{diagnostic}");
        assert!(diagnostic.contains("class=SameCutStrong"), "{diagnostic}");
        assert!(diagnostic.contains("segment=0"), "{diagnostic}");
        assert!(diagnostic.contains("local_offset=5000ms"), "{diagnostic}");
    }

    #[test]
    fn executable_version_probe_times_out() {
        let (executable, args) = slow_probe_command();
        let error = probe_executable_output_with_timeout(
            &executable,
            &args,
            std::time::Duration::from_millis(75),
        )
        .expect_err("slow probe command should time out");

        assert!(error.contains("timed out"), "{error}");
    }

    #[cfg(windows)]
    fn slow_probe_command() -> (PathBuf, Vec<&'static str>) {
        (
            PathBuf::from("powershell.exe"),
            vec!["-NoProfile", "-Command", "Start-Sleep -Seconds 2"],
        )
    }

    #[cfg(not(windows))]
    fn slow_probe_command() -> (PathBuf, Vec<&'static str>) {
        (PathBuf::from("/bin/sh"), vec!["-c", "sleep 2"])
    }

    #[test]
    fn offset_aware_retrieval_prefers_body_span_over_intro_outro_edges() {
        let root = unique_media_match_test_root("v3-offset-body-span");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let edge = vec![
            (1_000, 0, 4),
            (1_001, 30_000, 4),
            (1_002, 60_000, 4),
            (1_003, 1_100_000, 4),
            (1_004, 1_130_000, 4),
            (1_005, 1_160_000, 4),
        ];
        let body = (0..6)
            .map(|index| (2_000 + index, 360_000 + (index * 60_000), 4))
            .collect::<Vec<_>>();
        let mut query_anchors = edge.clone();
        query_anchors.extend(body.clone());
        let query = v3_record_with_audio_anchors("query.mkv", &query_anchors);
        let edge_candidate = v3_record_with_audio_anchors(
            "edge-candidate.mkv",
            &edge
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        let body_candidate = v3_record_with_audio_anchors(
            "body-candidate.mkv",
            &body
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        for record in [&query, &edge_candidate, &body_candidate] {
            save_media_match_record_to_index(&connection, record).expect("record should save");
        }

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("candidate lookup should run");

        assert_eq!(
            candidates.first(),
            Some(&body_candidate.identity.normalized_path)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn offset_aware_retrieval_is_deterministic_on_ties() {
        let root = unique_media_match_test_root("v3-offset-ties");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let query_anchors = (0..6)
            .map(|index| (3_000 + index, 120_000 + (index * 60_000), 4))
            .collect::<Vec<_>>();
        let query = v3_record_with_audio_anchors("query.mkv", &query_anchors);
        let first = v3_record_with_audio_anchors(
            "a-first.mkv",
            &query_anchors
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        let second = v3_record_with_audio_anchors(
            "b-second.mkv",
            &query_anchors
                .iter()
                .map(|(bucket, t_ms, weight)| (*bucket, *t_ms + 5_000, *weight))
                .collect::<Vec<_>>(),
        );
        for record in [&query, &first, &second] {
            save_media_match_record_to_index(&connection, record).expect("record should save");
        }

        let candidates = media_match_anchor_candidate_paths(
            &root,
            &query.identity.normalized_path,
            &query.extraction_settings,
        )
        .expect("candidate lookup should run");

        assert_eq!(candidates.first(), Some(&first.identity.normalized_path));
        let _ = std::fs::remove_dir_all(&root);
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
    fn media_match_index_build_abort_leaves_previous_sqlite_index_untouched() {
        let root = unique_media_match_test_root("restore");
        let mut previous_cache = MediaMatchCache::default();
        previous_cache.insert(fake_media_match_record("previous.mkv"));
        save_media_match_cache(&root, &previous_cache).expect("previous cache should be written");

        let transaction =
            prepare_media_match_index_rebuild_backup(&root).expect("backup should be prepared");
        let staging_root = transaction.staging_app_root().to_path_buf();
        clear_persisted_media_match_cache_at_root(&staging_root)
            .expect("staged index should be replaceable");
        let mut partial_cache = MediaMatchCache::default();
        partial_cache.insert(fake_media_match_record("partial.mkv"));
        save_media_match_cache(&staging_root, &partial_cache)
            .expect("partial staged cache should be written");

        transaction.abort().expect("staged rebuild should abort");
        let restored =
            load_media_match_cache_for_settings(&root, &MediaExtractionSettings::default())
                .expect("live cache should remain loadable");

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
    fn media_match_index_build_abort_leaves_missing_live_index_missing() {
        let root = unique_media_match_test_root("restore-empty");
        let transaction =
            prepare_media_match_index_rebuild_backup(&root).expect("backup should be prepared");
        let staging_root = transaction.staging_app_root().to_path_buf();

        let mut partial_cache = MediaMatchCache::default();
        partial_cache.insert(fake_media_match_record("partial.mkv"));
        save_media_match_cache(&staging_root, &partial_cache)
            .expect("partial staged cache should be written");
        assert!(!managed_media_match_index_path(&root).exists());

        transaction.abort().expect("staged rebuild should abort");
        assert!(!managed_media_match_index_path(&root).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_sqlite_index_tracks_fixed_sampled_fast_profile() {
        let root = unique_media_match_test_root("sqlite-fixed-profile");
        let mut cache = MediaMatchCache::default();
        cache.insert(fake_media_match_record("episode.mkv"));
        save_media_match_cache(&root, &cache).expect("cache should be written");

        let summary = open_media_match_index_session(&root)
            .expect("media index should open")
            .summary(&MediaExtractionSettings::sampled_fast_audio_index_v3())
            .expect("index summary should load");
        assert_eq!(summary.inventory_count, 1);
        assert_eq!(summary.fixed_settings_fingerprint_count, 1);
        assert_eq!(
            load_media_match_cache_for_settings(
                &root,
                &MediaExtractionSettings::sampled_fast_audio_index_v3()
            )
            .expect("fixed sampled-fast cache should load")
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
        let mut altered_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        altered_settings.sampled_audio_policy.policy_version = altered_settings
            .sampled_audio_policy
            .policy_version
            .saturating_add(1);
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
            load_media_match_cache_for_settings(
                &root,
                &MediaExtractionSettings::sampled_fast_audio_index_v3()
            )
            .is_none(),
            "same profile rows with a different settings hash must not be reused"
        );
        assert!(
            media_match_record_for_path(
                &root,
                media_path.to_str().expect("test path should be UTF-8"),
                &MediaExtractionSettings::sampled_fast_audio_index_v3(),
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
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
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

        std::fs::write(&candidate_path, b"candidate-v3-with-new-size")
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
    fn media_match_save_and_loads_valid_audio_v3_landmarks() {
        let root = unique_media_match_test_root("valid-audio-v3");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let mut record = fake_media_match_record("valid-audio-v3.mkv");
        record.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        record.audio_anchors = vec![AudioAnchor {
            bucket: 42,
            t_ms: 1_000,
            weight: 3,
        }];

        save_media_match_record_to_index(&connection, &record)
            .expect("valid audio V3 landmark should save");
        let cache = load_media_match_cache_for_settings(
            &root,
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        )
        .expect("valid audio V3 cache should load");
        let loaded = cache
            .records
            .get(&record.identity.normalized_path)
            .expect("saved record should be loaded");

        assert_eq!(
            loaded.audio_anchors.first().map(|anchor| anchor.bucket),
            Some(42)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "requires ffmpeg/ffprobe in SOROTTE_MEDIA_MATCH_FFMPEG/SOROTTE_MEDIA_MATCH_FFPROBE or PATH"]
    fn v3_manifest_harness_runs_small_synthetic_case() {
        let Some(ffmpeg) = test_ffmpeg_path() else {
            eprintln!("skipping ignored ffmpeg test: ffmpeg is not available");
            return;
        };
        let Some(ffprobe) = test_ffprobe_path() else {
            eprintln!("skipping ignored ffmpeg test: ffprobe is not available");
            return;
        };
        let root = unique_media_match_test_root("v3-manifest-harness");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let query = media_dir.join("query.mkv");
        let candidate = media_dir.join("candidate.mkv");
        generate_v3_synthetic_media(&ffmpeg, &query, 440, 23);
        std::fs::copy(&query, &candidate).expect("candidate media should be copied");
        let tools = MediaMatchToolPaths { ffmpeg, ffprobe };
        let manifest = serde_json::json!({
            "profile": "audio-constellation-v3",
            "baseDir": "media",
            "cases": [{
                "name": "copied-synthetic-media",
                "query": "query.mkv",
                "candidates": [{
                    "path": "candidate.mkv",
                    "minimumTier": "Probable",
                    "mustBeRetrieved": true
                }]
            }]
        });

        let manifest = sorotte_media_match::media_match_v3_diagnostic_manifest_from_json(
            &manifest.to_string(),
        )
        .expect("manifest should parse");
        let report = sorotte_media_match::run_media_match_v3_diagnostic_manifest(
            &manifest,
            &sorotte_media_match::MediaMatchV3DiagnosticRunOptions {
                manifest_dir: root.clone(),
                cache_root: root.join("diagnostic-cache"),
                cache_retained: true,
                refresh_cache: false,
                index_mode: sorotte_media_match::MediaMatchV3DiagnosticIndexMode::SampledFast,
                retrieval_benchmark_only: false,
                tools,
                generated_at_unix_millis: Some(123),
            },
        )
        .expect("diagnostic manifest should run");
        let report_json =
            sorotte_media_match::media_match_v3_diagnostic_manifest_report_json(&report)
                .expect("report should serialize");
        let report: serde_json::Value =
            serde_json::from_str(&report_json).expect("report JSON should parse");
        let candidate = &report["cases"][0]["candidates"][0];

        assert_eq!(report["algorithmVersion"], MEDIA_MATCH_ALGORITHM_VERSION);
        assert_eq!(candidate["expectationPassed"], true);
        assert_eq!(candidate["retrieved"], true);
        assert!(
            report["cases"][0]["query"]["diagnostics"]["audioBlobBytes"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        assert!(
            report["cases"][0]["retrieval"]["stats"]["queryBucketsTotal"]
                .as_i64()
                .unwrap_or_default()
                >= 0
        );
        assert!(candidate["decision"].get("class").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn test_ffmpeg_path() -> Option<PathBuf> {
        test_tool_path("SOROTTE_MEDIA_MATCH_FFMPEG", "ffmpeg")
    }

    fn test_ffprobe_path() -> Option<PathBuf> {
        test_tool_path("SOROTTE_MEDIA_MATCH_FFPROBE", "ffprobe")
    }

    fn test_tool_path(env_key: &str, default_name: &str) -> Option<PathBuf> {
        let path = std::env::var_os(env_key)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default_name));
        let status = Command::new(&path)
            .arg("-version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        status.success().then_some(path)
    }

    fn generate_v3_synthetic_media(ffmpeg: &Path, path: &Path, frequency_hz: u32, crf: u8) {
        let status = Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=1:duration=30",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency_hz}:sample_rate=44100:duration=30"),
                "-shortest",
                "-c:v",
                "libx264",
                "-crf",
                &crf.to_string(),
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(path)
            .status()
            .expect("ffmpeg should create synthetic media");
        assert!(status.success(), "ffmpeg fixture generation failed");
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
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
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
    fn media_match_inventory_preserves_records_when_configured_root_is_temporarily_missing() {
        let root = unique_media_match_test_root("temporarily-missing-inventory-root");
        let media_dir = root.join("media");
        let offline_media_dir = root.join("media-offline");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let media_path = media_dir.join("episode.mkv");
        std::fs::write(&media_path, b"episode").expect("media should be written");
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let record = fake_media_match_record_for_file(&media_path, extraction_settings.clone());
        let normalized_path = record.identity.normalized_path.clone();
        let mut cache = MediaMatchCache::default();
        cache.insert(record);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        std::fs::rename(&media_dir, &offline_media_dir)
            .expect("configured root should become temporarily unavailable");
        rebuild_persisted_media_match_inventory_for_tests(&root, std::slice::from_ref(&media_dir))
            .expect("temporarily missing root should not make the inventory rebuild fail");

        let cache = load_media_match_cache_for_settings(&root, &extraction_settings)
            .expect("cached record should still load");
        assert!(
            cache.records.contains_key(&normalized_path),
            "a temporarily missing configured root must not be treated as a successful empty scan"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_inventory_prunes_healthy_roots_while_retaining_an_offline_root() {
        let root = unique_media_match_test_root("mixed-online-offline-inventory-roots");
        let online_dir = root.join("online");
        let offline_dir = root.join("offline");
        let moved_offline_dir = root.join("offline-unmounted");
        std::fs::create_dir_all(&online_dir).expect("online media dir should be created");
        std::fs::create_dir_all(&offline_dir).expect("offline media dir should be created");
        let deleted_online_path = online_dir.join("deleted.mkv");
        let retained_offline_path = offline_dir.join("retained.mkv");
        std::fs::write(&deleted_online_path, b"online").expect("online media should be written");
        std::fs::write(&retained_offline_path, b"offline")
            .expect("offline media should be written");
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let deleted_online =
            fake_media_match_record_for_file(&deleted_online_path, extraction_settings.clone());
        let retained_offline =
            fake_media_match_record_for_file(&retained_offline_path, extraction_settings.clone());
        let deleted_online_key = deleted_online.identity.normalized_path.clone();
        let retained_offline_key = retained_offline.identity.normalized_path.clone();
        let mut cache = MediaMatchCache::default();
        cache.insert(deleted_online);
        cache.insert(retained_offline);
        save_media_match_cache(&root, &cache).expect("cache should be written");

        std::fs::remove_file(&deleted_online_path).expect("online file should be deleted");
        std::fs::rename(&offline_dir, &moved_offline_dir)
            .expect("offline root should become unavailable");
        rebuild_persisted_media_match_inventory_for_tests(
            &root,
            &[online_dir.clone(), offline_dir.clone()],
        )
        .expect("mixed-root rebuild should succeed");

        let cache = load_media_match_cache_for_settings(&root, &extraction_settings)
            .expect("retained cache should load");
        assert!(
            !cache.records.contains_key(&deleted_online_key),
            "a healthy scanned root must continue pruning genuinely deleted files"
        );
        assert!(
            cache.records.contains_key(&retained_offline_key),
            "an unavailable root must retain its cached records"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_inventory_prune_refreshes_v3_anchor_stats() {
        let root = unique_media_match_test_root("deleted-inventory-stats");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let kept_path = media_dir.join("kept.mkv");
        let removed_path = media_dir.join("removed.mkv");
        std::fs::write(&kept_path, b"kept").expect("kept media should be written");
        std::fs::write(&removed_path, b"removed").expect("removed media should be written");
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let mut kept = fake_media_match_record_for_file(&kept_path, extraction_settings.clone());
        kept.audio_anchors = vec![AudioAnchor {
            bucket: 500,
            t_ms: 10_000,
            weight: 1,
        }];
        let mut removed =
            fake_media_match_record_for_file(&removed_path, extraction_settings.clone());
        removed.audio_anchors = vec![
            AudioAnchor {
                bucket: 500,
                t_ms: 11_000,
                weight: 1,
            },
            AudioAnchor {
                bucket: 501,
                t_ms: 12_000,
                weight: 1,
            },
        ];
        let mut cache = MediaMatchCache::default();
        cache.insert(kept.clone());
        cache.insert(removed);
        save_media_match_cache(&root, &cache).expect("cache should be written");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        connection
            .refresh_all_anchor_stats(current_unix_millis() as i64)
            .expect("initial stats refresh should succeed");
        let settings_hash = media_extraction_settings_hash(&extraction_settings);
        let shared_before = v3_audio_bucket_document_frequency(&connection, &settings_hash, 500);
        let removed_only_before =
            v3_audio_bucket_document_frequency(&connection, &settings_hash, 501);
        assert_eq!(shared_before, Some(2));
        assert_eq!(removed_only_before, Some(1));

        std::fs::remove_file(&removed_path).expect("removed media should be deleted");
        inventory_media_match_candidates(
            &root,
            std::slice::from_ref(&media_dir),
            std::slice::from_ref(&kept_path),
            None,
        )
        .expect("inventory should prune deleted files and mark stats dirty");
        assert!(
            connection
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "inventory pruning should defer expensive all-settings stats refresh"
        );
        media_match_anchor_candidate_paths(
            &root,
            &kept.identity.normalized_path,
            &extraction_settings,
        )
        .expect("candidate lookup should refresh dirty anchor stats");

        let refreshed_connection =
            open_media_match_index_session(&root).expect("media index should reopen");
        let shared_after =
            v3_audio_bucket_document_frequency(&refreshed_connection, &settings_hash, 500);
        let removed_only_after =
            v3_audio_bucket_document_frequency(&refreshed_connection, &settings_hash, 501);
        assert_eq!(shared_after, Some(1));
        assert_eq!(
            removed_only_after,
            Some(0),
            "anchor stats for pruned-only buckets should be refreshed to zero"
        );
        assert!(
            !refreshed_connection
                .anchor_stats_dirty(&settings_hash)
                .expect("dirty marker should load"),
            "candidate lookup should clear the dirty marker after refreshing stats"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_v3_audio_blob_storage_stays_under_two_kb_per_file() {
        let root = unique_media_match_test_root("v3-audio-size");
        let mut record = fake_media_match_record("episode.mkv");
        record.extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        record.audio_anchors = (0..96)
            .map(|index| AudioAnchor {
                bucket: index,
                t_ms: index * 10_000,
                weight: 1,
            })
            .collect();
        let mut cache = MediaMatchCache::default();
        cache.insert(record);
        save_media_match_cache(&root, &cache).expect("cache should be written");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let blob_bytes = connection
            .summary(&MediaExtractionSettings::sampled_fast_audio_index_v3())
            .expect("index summary should load")
            .v3_audio_blob_bytes;

        assert!(
            blob_bytes <= 2_048,
            "v3 audio profile blob bytes should stay under 2KB, got {blob_bytes}"
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
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            None,
            |_| {},
        )
        .expect("inventory-only scan should not require media tools");
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let summary = connection
            .summary(&MediaExtractionSettings::sampled_fast_audio_index_v3())
            .expect("index summary should load");

        assert_eq!(summary.inventory_count, 2);
        assert_eq!(summary.v3_fingerprint_row_count, 0);
        assert!(result.message.contains("inventoried 2 discovered files"));
        assert!(result.message.contains("No active local media path"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sampled_background_worker_count_is_capped() {
        let sampled = media_match_extraction_worker_count(
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );

        assert!((1..=8).contains(&sampled), "sampled workers={sampled}");
    }

    #[test]
    fn parallel_sampled_rebuild_cancel_reports_unscheduled_files_without_invoking_tools() {
        let cancel = AtomicBool::new(true);
        let tools = MediaMatchToolPaths {
            ffmpeg: PathBuf::from("ffmpeg-not-used"),
            ffprobe: PathBuf::from("ffprobe-not-used"),
        };

        let (results, stats) = parallel_fresh_media_fingerprints(
            vec![PathBuf::from("one.mkv"), PathBuf::from("two.mkv")],
            &tools,
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            Some(&cancel),
            |_, _| {},
        )
        .expect("pre-cancelled worker pool should report cancelled work, not tool errors");

        assert!(results.is_empty());
        assert_eq!(stats.cancelled_file_count, 2);
        assert_eq!(stats.files_indexed, 0);
    }

    #[test]
    fn parallel_sampled_rebuild_reports_progress_for_completed_outputs() {
        let root = unique_media_match_test_root("parallel-progress");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let first = media_dir.join("one.mkv");
        let second = media_dir.join("two.mkv");
        std::fs::write(&first, vec![1u8; 2000]).expect("first media file should be written");
        std::fs::write(&second, vec![2u8; 2000]).expect("second media file should be written");
        let tools = MediaMatchToolPaths {
            ffmpeg: PathBuf::from("ffmpeg-not-used"),
            ffprobe: PathBuf::from("ffprobe-not-used"),
        };
        let mut updates = Vec::new();

        let (results, stats) = parallel_fresh_media_fingerprints(
            vec![first, second],
            &tools,
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            None,
            |completed, path| updates.push((completed, path.to_path_buf())),
        )
        .expect("worker pool should return failed outputs for missing tools");

        assert_eq!(results.len(), 2);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].0, 1);
        assert_eq!(updates[1].0, 2);
        assert_eq!(stats.files_indexed, 0);
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
            &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );

        assert_eq!(
            current_decision.as_deref(),
            Some("exact: current local file is indexed")
        );
        let nearest_match = nearest_match.expect("nearest candidate should be reported");
        assert!(
            nearest_match.contains(
                "No alternate indexed match; nearest other: episode-nearest.mkv (reject: sampled-fast audio did not align)"
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
            last_evidence.contains("no coherent sampled-fast audio offset"),
            "{last_evidence}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn media_match_rebuild_with_no_fresh_work_does_not_rewrite_sqlite_records() {
        let root = unique_media_match_test_root("no-op-rebuild");
        let media_dir = root.join("media");
        std::fs::create_dir_all(&media_dir).expect("media dir should be created");
        let media_path = media_dir.join("episode.mkv");
        std::fs::write(&media_path, vec![42u8; 2000]).expect("test media file should be written");
        let extraction_settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
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
                .starts_with("inventory: 1, audio-v3(all settings): 1")
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
        };
        let cancel = AtomicBool::new(true);
        let result = rebuild_persisted_media_match_candidates_with_progress_and_cancel(
            MediaMatchCandidateRebuildRequest {
                root: &root,
                candidates: vec![media_path.clone()],
                current_player_path: media_path.to_str(),
                settings: &settings,
                tools: &tools,
                extraction_settings: &MediaExtractionSettings::sampled_fast_audio_index_v3(),
                cancel_flag: Some(&cancel),
            },
            |_| {},
        );

        assert!(result.is_err());
        let connection = open_media_match_index_session(&root).expect("media index should open");
        let fingerprints = connection
            .summary(&MediaExtractionSettings::sampled_fast_audio_index_v3())
            .expect("index summary should load")
            .v3_fingerprint_row_count;
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
    fn media_match_filename_profile_uses_parent_arc_context_for_monogatari() {
        let query = media_match_filename_profile(Path::new(
            "[Coalgirls]_Otorimonogatari_01_(BD_1080p).mkv",
        ));
        let matching_candidate = media_match_filename_profile(Path::new(
            "C:/Users/shaun/Documents/workspace/[MTBB-Minis] Monogatari Series (BD 1080p)/08 - Otorimonogatari/[MTBB-Minis] Monogatari Series Second Season - 10 [BD 1080p].mkv",
        ));
        let wrong_candidate = media_match_filename_profile(Path::new(
            "C:/Users/shaun/Documents/workspace/[MTBB-Minis] Monogatari Series (BD 1080p)/07 - Onimonogatari/[MTBB-Minis] Monogatari Series Second Season - 09 [BD 1080p].mkv",
        ));

        assert!(matching_candidate.series_tokens.contains("otorimonogatari"));
        assert!(
            media_match_filename_score(&query, &matching_candidate)
                > media_match_filename_score(&query, &wrong_candidate)
        );
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
    fn media_match_remote_prefilter_keeps_monogatari_parent_arc_candidate() {
        let matching_path = PathBuf::from(
            "C:/Users/shaun/Documents/workspace/[MTBB-Minis] Monogatari Series (BD 1080p)/08 - Otorimonogatari/[MTBB-Minis] Monogatari Series Second Season - 10 [BD 1080p].mkv",
        );
        let mut candidates = vec![matching_path.clone()];
        candidates.extend((0..MEDIA_MATCH_PREFILTER_THRESHOLD + 10).map(|index| {
            PathBuf::from(format!(
                "C:/Users/shaun/Documents/workspace/[MTBB-Minis] Monogatari Series (BD 1080p)/07 - Onimonogatari/[MTBB-Minis] Monogatari Series Second Season - {index:02} [BD 1080p].mkv"
            ))
        }));

        let selection = select_remote_media_match_candidates(
            &candidates,
            "[Coalgirls]_Otorimonogatari_01_(BD_1080p).mkv",
        );

        assert!(selection.prefiltered);
        assert!(selection.paths.contains(&matching_path));
        assert_eq!(selection.paths.first(), Some(&matching_path));
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
