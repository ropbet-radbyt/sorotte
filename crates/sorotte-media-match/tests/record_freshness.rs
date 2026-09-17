use sorotte_media_match::{
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexInventoryEntry, MediaIndexRecordReader, MediaIndexService,
};
use std::{path::PathBuf, time::SystemTime};

struct Fixture {
    root: PathBuf,
    record: MediaFingerprintRecord,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sorotte-file-identity-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let record = MediaFingerprintRecord {
            identity: MediaFileIdentity::new(root.join("episode.mkv"), 10_000, 4096),
            algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings: MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(120.0),
            container_fingerprint: "original-edition".to_owned(),
            audio_anchors: (0..48)
                .map(|index| AudioAnchor {
                    bucket: 1000 + index,
                    t_ms: index * 2000,
                    weight: 10,
                })
                .collect(),
            audio_error: None,
        };
        MediaIndexService::new(&root)
            .open()
            .unwrap()
            .save_record(&record, None)
            .unwrap();
        Self { root, record }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn session_requires_revalidation_after_timestamp_drift() {
    let fixture = Fixture::new("session");
    let identity = &fixture.record.identity;
    let loaded = MediaIndexService::new(&fixture.root)
        .open()
        .unwrap()
        .load_record(
            &identity.normalized_path,
            &fixture.record.extraction_settings,
            identity.modified_unix_millis + 1000,
            identity.size_bytes,
        )
        .unwrap();
    assert_eq!(
        loaded, None,
        "changed metadata requires revalidation before a cached fingerprint is reused"
    );
}

#[test]
fn record_reader_requires_revalidation_after_timestamp_drift() {
    let fixture = Fixture::new("reader");
    let identity = &fixture.record.identity;
    let mut reader = MediaIndexRecordReader::new(&fixture.root);
    assert!(
        reader
            .load_record(
                &identity.normalized_path,
                &fixture.record.extraction_settings,
                identity.modified_unix_millis,
                identity.size_bytes
            )
            .unwrap()
            .is_some()
    );
    let loaded = reader
        .load_record(
            &identity.normalized_path,
            &fixture.record.extraction_settings,
            identity.modified_unix_millis + 1000,
            identity.size_bytes,
        )
        .unwrap();
    assert_eq!(
        loaded, None,
        "the reusable reader must withhold an unvalidated fingerprint"
    );
}

#[test]
fn unchanged_identity_control() {
    let fixture = Fixture::new("unchanged");
    let identity = &fixture.record.identity;
    let loaded = MediaIndexService::new(&fixture.root)
        .open()
        .unwrap()
        .load_record(
            &identity.normalized_path,
            &fixture.record.extraction_settings,
            identity.modified_unix_millis,
            identity.size_bytes,
        )
        .unwrap()
        .unwrap();
    assert_eq!(loaded.identity, *identity);
    assert_eq!(loaded.audio_anchors, fixture.record.audio_anchors);
}

#[test]
fn changed_size_control() {
    let fixture = Fixture::new("size");
    let identity = &fixture.record.identity;
    assert!(
        MediaIndexService::new(&fixture.root)
            .open()
            .unwrap()
            .load_record(
                &identity.normalized_path,
                &fixture.record.extraction_settings,
                identity.modified_unix_millis + 1000,
                identity.size_bytes + 1
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn explicit_inventory_refresh_invalidates_same_size_edit_control() {
    let fixture = Fixture::new("refresh");
    let identity = &fixture.record.identity;
    let session = MediaIndexService::new(&fixture.root).open().unwrap();
    session
        .refresh_inventory(
            &[MediaIndexInventoryEntry::new(
                &identity.normalized_path,
                identity.modified_unix_millis + 1000,
                identity.size_bytes,
            )],
            std::slice::from_ref(&identity.normalized_path),
            &[],
            || false,
        )
        .unwrap();
    assert!(
        session
            .load_record(
                &identity.normalized_path,
                &fixture.record.extraction_settings,
                identity.modified_unix_millis + 1000,
                identity.size_bytes
            )
            .unwrap()
            .is_none()
    );
}
