use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::{
    MediaExtractionSettings, MediaFingerprintRecord, MediaMatchCache, MediaMatchV3RetrievalStats,
    MediaMatchV3SaveStats, delete_media_match_v3_file_and_fingerprints,
    delete_media_match_v3_fingerprints_and_anchors, load_media_match_v3_cache_for_settings,
    load_media_match_v3_record_for_path, media_match_v3_anchor_candidate_paths_with_stats,
    media_match_v3_index_path, open_media_match_v3_index, refresh_anchor_stats_v3,
    save_media_match_v3_record, save_media_match_v3_record_with_stats,
};

/// Owns the location and lifecycle of one media-match index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIndexService {
    root: PathBuf,
}

impl MediaIndexService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index_path(&self) -> PathBuf {
        media_match_v3_index_path(&self.root)
    }

    pub fn open(&self) -> Result<MediaIndexSession, String> {
        open_media_match_v3_index(&self.root).map(|connection| MediaIndexSession { connection })
    }
}

/// An initialized media-index connection with semantic operations.
pub struct MediaIndexSession {
    connection: Connection,
}

impl MediaIndexSession {
    pub fn load_cache(
        &self,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<MediaMatchCache, String> {
        load_media_match_v3_cache_for_settings(&self.connection, extraction_settings)
    }

    pub fn load_record(
        &self,
        normalized_path: &str,
        extraction_settings: &MediaExtractionSettings,
        modified_unix_millis: u64,
        size_bytes: u64,
    ) -> Result<Option<MediaFingerprintRecord>, String> {
        load_media_match_v3_record_for_path(
            &self.connection,
            normalized_path,
            extraction_settings,
            modified_unix_millis,
            size_bytes,
        )
    }

    pub fn save_record(
        &self,
        record: &MediaFingerprintRecord,
        error: Option<&str>,
    ) -> Result<(), String> {
        save_media_match_v3_record(&self.connection, record, error)
    }

    pub fn save_record_with_stats(
        &self,
        record: &MediaFingerprintRecord,
        now_unix_millis: i64,
    ) -> Result<MediaMatchV3SaveStats, String> {
        save_media_match_v3_record_with_stats(&self.connection, record, now_unix_millis)
    }

    pub fn anchor_candidate_paths(
        &self,
        normalized_current_path: &str,
        extraction_settings: &MediaExtractionSettings,
    ) -> Result<(Vec<String>, MediaMatchV3RetrievalStats), String> {
        media_match_v3_anchor_candidate_paths_with_stats(
            &self.connection,
            normalized_current_path,
            extraction_settings,
        )
    }

    pub fn refresh_anchor_stats(
        &self,
        settings_hash: &[u8; 32],
        now_unix_millis: i64,
    ) -> Result<(), String> {
        refresh_anchor_stats_v3(&self.connection, settings_hash, now_unix_millis)
    }

    pub fn delete_fingerprints(&self, normalized_path: &str) -> Result<(), String> {
        delete_media_match_v3_fingerprints_and_anchors(&self.connection, normalized_path)
    }

    pub fn delete_file(&self, normalized_path: &str) -> Result<(), String> {
        delete_media_match_v3_file_and_fingerprints(&self.connection, normalized_path)
    }
}
