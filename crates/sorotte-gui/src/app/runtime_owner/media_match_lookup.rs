use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sorotte_media_match::{
    MediaExtractionSettings, MediaFingerprintRecord, MediaIndexRecordReader,
    media_match_wire_value_from_records, normalize_media_path,
};

use crate::app::media_match_support::{
    MediaMatchInventoryExactResolution, media_match_inventory_exact_resolution_from_paths,
};
use crate::app::runtime_queue::GuiQueuedRuntimeBridgeHandle;

const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, PartialEq)]
pub(in crate::app) struct FingerprintLookupValue {
    pub(in crate::app) record: MediaFingerprintRecord,
    pub(in crate::app) wire_value: Option<serde_json::Value>,
}

#[derive(Clone, PartialEq)]
pub(in crate::app) enum FingerprintLookup {
    Pending,
    Missing,
    Present(Arc<FingerprintLookupValue>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum InventoryLookup {
    Pending,
    Ready(Option<MediaMatchInventoryExactResolution>),
    Failed(String),
}

#[derive(Clone, PartialEq, Eq)]
struct InventoryKey {
    root: PathBuf,
    search_roots: Vec<PathBuf>,
    targets: Vec<String>,
    epoch: u64,
}

struct InventoryReply {
    key: InventoryKey,
    result: InventoryLookup,
    completed_at: Instant,
}

#[derive(Clone, PartialEq, Eq)]
struct LookupKey {
    root: PathBuf,
    path: String,
    settings: MediaExtractionSettings,
    modified: SystemTime,
    created: Option<SystemTime>,
    bytes: u64,
    epoch: u64,
}

struct LookupReply {
    key: LookupKey,
    result: FingerprintLookup,
    completed_at: Instant,
}

enum LookupRequest {
    Read(LookupKey),
    Inventory(InventoryKey),
    Clear(PathBuf),
}

enum LookupEvent {
    Read(LookupReply),
    Inventory(InventoryReply),
    Cleared(PathBuf, Result<(), String>),
}

struct LookupWorker {
    requests: mpsc::SyncSender<LookupRequest>,
    replies: mpsc::Receiver<LookupEvent>,
}

#[derive(Default)]
pub(in crate::app) struct GuiMediaMatchRecordLookup {
    worker: Option<LookupWorker>,
    cached: Option<LookupReply>,
    inventory: std::collections::VecDeque<InventoryReply>,
    in_flight: bool,
    epoch: u64,
    changed: bool,
    pending_clear: Option<PathBuf>,
    clear_running: bool,
    clear_result: Option<(PathBuf, Result<(), String>)>,
    wake: GuiQueuedRuntimeBridgeHandle,
}

impl GuiMediaMatchRecordLookup {
    #[cfg(test)]
    pub(in crate::app) fn wait_for_inventory_test(
        &mut self,
        root: &Path,
        search_roots: &[PathBuf],
        targets: &[String],
    ) -> InventoryLookup {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = self.lookup_inventory(root, search_roots, targets);
            if result != InventoryLookup::Pending {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "inventory lookup did not complete"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[cfg(test)]
    pub(in crate::app) fn wait_for_test(&mut self, root: &Path, path: &str) -> FingerprintLookup {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = self.lookup(
                root,
                path,
                &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            );
            if result != FingerprintLookup::Pending {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "fingerprint lookup did not complete"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(in crate::app) fn set_wake(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        self.wake = handle.clone();
    }

    pub(in crate::app) fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cached = None;
        self.inventory.clear();
        self.changed = true;
    }

    pub(in crate::app) fn take_changed(&mut self) -> bool {
        self.poll();
        std::mem::take(&mut self.changed)
    }

    pub(in crate::app) fn is_clearing(&self) -> bool {
        self.pending_clear.is_some()
    }

    pub(in crate::app) fn clear(&mut self, root: PathBuf) -> Result<(), String> {
        if self
            .pending_clear
            .as_ref()
            .is_some_and(|pending| pending != &root)
        {
            return Err("Another Media Match cache clear is still running".to_owned());
        }
        self.invalidate();
        self.pending_clear = Some(root);
        self.poll();
        Ok(())
    }

    pub(in crate::app) fn take_clear_result(&mut self) -> Option<(PathBuf, Result<(), String>)> {
        self.poll();
        self.clear_result.take()
    }

    fn start_pending_clear(&mut self) {
        if self.in_flight || self.clear_running {
            return;
        }
        let Some(root) = self.pending_clear.clone() else {
            return;
        };
        if self.worker.is_none() {
            match spawn_lookup_worker(self.wake.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.pending_clear = None;
                    self.clear_result = Some((root, Err(error.to_string())));
                    return;
                }
            }
        }
        match self
            .worker
            .as_ref()
            .expect("worker started above")
            .requests
            .try_send(LookupRequest::Clear(root.clone()))
        {
            Ok(()) => self.clear_running = true,
            Err(error) => {
                self.pending_clear = None;
                self.clear_result = Some((root, Err(error.to_string())));
            }
        }
    }

    fn poll(&mut self) {
        let event = self.worker.as_ref().map(|worker| worker.replies.try_recv());
        match event {
            Some(Ok(LookupEvent::Inventory(reply))) => {
                self.in_flight = false;
                if reply.key.epoch == self.epoch {
                    let old = self
                        .inventory
                        .iter()
                        .position(|cached| cached.key == reply.key)
                        .and_then(|index| self.inventory.remove(index));
                    self.changed |= old
                        .as_ref()
                        .is_none_or(|cached| cached.result != reply.result);
                    // Alias queries share one worker and a small bounded cache.
                    if self.inventory.len() == 16 {
                        self.inventory.pop_front();
                    }
                    self.inventory.push_back(reply);
                }
            }
            Some(Ok(LookupEvent::Read(reply))) => {
                self.in_flight = false;
                if reply.key.epoch == self.epoch {
                    self.changed |= self.cached.as_ref().is_none_or(|cached| {
                        cached.key != reply.key || cached.result != reply.result
                    });
                    self.cached = Some(reply);
                }
            }
            Some(Ok(LookupEvent::Cleared(root, result))) => {
                self.clear_running = false;
                self.pending_clear = None;
                self.clear_result = Some((root, result));
                self.invalidate();
            }
            Some(Err(mpsc::TryRecvError::Empty)) | None => {}
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.worker = None;
                self.in_flight = false;
                self.invalidate();
                if let Some(root) = self.pending_clear.take() {
                    self.clear_running = false;
                    self.clear_result =
                        Some((root, Err("Media Match cache worker stopped".to_owned())));
                }
            }
        }
        self.start_pending_clear();
    }

    pub(in crate::app) fn lookup_inventory(
        &mut self,
        root: &Path,
        search_roots: &[PathBuf],
        targets: &[String],
    ) -> InventoryLookup {
        self.poll();
        if self.is_clearing() {
            return InventoryLookup::Pending;
        }
        if search_roots.is_empty() || targets.is_empty() {
            return InventoryLookup::Ready(None);
        }
        let key = InventoryKey {
            root: root.to_owned(),
            search_roots: search_roots.to_vec(),
            targets: targets.to_vec(),
            epoch: self.epoch,
        };
        let cached = self.inventory.iter().find(|reply| reply.key == key);
        let result = cached.map_or(InventoryLookup::Pending, |reply| reply.result.clone());
        let refresh = cached.is_none_or(|reply| reply.completed_at.elapsed() >= REFRESH_INTERVAL);
        if refresh && !self.in_flight {
            if self.worker.is_none() {
                match spawn_lookup_worker(self.wake.clone()) {
                    Ok(worker) => self.worker = Some(worker),
                    Err(error) => return InventoryLookup::Failed(error.to_string()),
                }
            }
            match self
                .worker
                .as_ref()
                .expect("worker started above")
                .requests
                .try_send(LookupRequest::Inventory(key))
            {
                Ok(()) => self.in_flight = true,
                Err(mpsc::TrySendError::Full(_)) => self.in_flight = true,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.worker = None;
                    return InventoryLookup::Failed("Media Match lookup worker stopped".to_owned());
                }
            }
        }
        result
    }

    pub(in crate::app) fn lookup(
        &mut self,
        root: &Path,
        path: &str,
        settings: &MediaExtractionSettings,
    ) -> FingerprintLookup {
        #[cfg(test)]
        let _latency_review_span = crate::app::latency_review_probe::span("mm.record.cached");
        self.poll();
        if self.is_clearing() {
            return FingerprintLookup::Pending;
        }
        // Bind a cached observation to the actual file, including sub-millisecond
        // changes. No database or activation-lock operation runs on this thread.
        let metadata = match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return FingerprintLookup::Missing,
            Err(error) => return FingerprintLookup::Failed(error.to_string()),
        };
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error) => return FingerprintLookup::Failed(error.to_string()),
        };
        let key = LookupKey {
            root: root.to_owned(),
            path: path.to_owned(),
            settings: settings.clone(),
            modified,
            created: metadata.created().ok(),
            bytes: metadata.len(),
            epoch: self.epoch,
        };
        let cached = self.cached.as_ref().filter(|reply| reply.key == key);
        let result = cached.map_or(FingerprintLookup::Pending, |reply| reply.result.clone());
        let refresh = cached.is_none_or(|reply| reply.completed_at.elapsed() >= REFRESH_INTERVAL);
        if refresh && !self.in_flight {
            if self.worker.is_none() {
                match spawn_lookup_worker(self.wake.clone()) {
                    Ok(worker) => self.worker = Some(worker),
                    Err(error) => return FingerprintLookup::Failed(error.to_string()),
                }
            }
            match self
                .worker
                .as_ref()
                .expect("worker started above")
                .requests
                .try_send(LookupRequest::Read(key))
            {
                Ok(()) => self.in_flight = true,
                Err(mpsc::TrySendError::Full(_)) => self.in_flight = true,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.worker = None;
                    return FingerprintLookup::Failed(
                        "Media Match lookup worker stopped".to_owned(),
                    );
                }
            }
        }
        result
    }
}

fn spawn_lookup_worker(wake: GuiQueuedRuntimeBridgeHandle) -> std::io::Result<LookupWorker> {
    let (requests, rx) = mpsc::sync_channel::<LookupRequest>(1);
    let (tx, replies) = mpsc::channel();
    thread::Builder::new().name("sorotte-media-match-record-reader".to_owned()).spawn(move || {
        let mut reader: Option<(PathBuf, MediaIndexRecordReader)> = None;
        while let Ok(request) = rx.recv() {
            let key = match request {
                LookupRequest::Read(key) => key,
                LookupRequest::Inventory(key) => {
                    if reader.as_ref().is_none_or(|(root, _)| root != &key.root) {
                        reader = Some((key.root.clone(), MediaIndexRecordReader::new(key.root.join("cache/media-match"))));
                    }
                    let result = match reader.as_mut().expect("reader initialized above").1.inventory_paths() {
                        Ok(rows) => InventoryLookup::Ready(media_match_inventory_exact_resolution_from_paths(rows, &key.search_roots, &key.targets)),
                        Err(error) => InventoryLookup::Failed(error),
                    };
                    if tx.send(LookupEvent::Inventory(InventoryReply { key, result, completed_at: Instant::now() })).is_err() { break; }
                    wake.notify_threaded_runtime_owner();
                    continue;
                }
                LookupRequest::Clear(root) => {
                    reader = None;
                    let result = crate::app::media_match_support::clear_persisted_media_match_cache_at_root(&root);
                    if tx.send(LookupEvent::Cleared(root, result)).is_err() { break; }
                    wake.notify_threaded_runtime_owner();
                    continue;
                }
            };
            if reader.as_ref().is_none_or(|(root, _)| root != &key.root) {
                reader = Some((key.root.clone(), MediaIndexRecordReader::new(key.root.join("cache/media-match"))));
            }
            let modified_millis = key.modified.duration_since(UNIX_EPOCH).unwrap_or_default()
                .as_millis().min(u128::from(u64::MAX)) as u64;
            let result = reader.as_mut().expect("reader initialized above").1.load_record(
                &normalize_media_path(&key.path), &key.settings, modified_millis, key.bytes,
            );
            let result = match result {
                Ok(Some(record)) => {
                    let wire_value = media_match_wire_value_from_records(std::slice::from_ref(&record));
                    FingerprintLookup::Present(Arc::new(FingerprintLookupValue { record, wire_value }))
                }
                Ok(None) => FingerprintLookup::Missing,
                Err(error) => FingerprintLookup::Failed(error),
            };
            if tx.send(LookupEvent::Read(LookupReply { key, result, completed_at: Instant::now() })).is_err() { break; }
            wake.notify_threaded_runtime_owner();
        }
    })?;
    Ok(LookupWorker { requests, replies })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_clear_blocks_reads_and_worker_disconnect_returns_a_terminal_error() {
        let root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let roots = [root.path().to_owned()];
        let targets = ["episode.mkv".to_owned()];
        let (requests, rx) = mpsc::sync_channel(1);
        let (tx, replies) = mpsc::channel();
        let mut lookup = GuiMediaMatchRecordLookup {
            worker: Some(LookupWorker { requests, replies }),
            ..Default::default()
        };
        assert_eq!(
            lookup.lookup_inventory(root.path(), &roots, &targets),
            InventoryLookup::Pending
        );
        assert!(matches!(
            rx.try_recv().unwrap(),
            LookupRequest::Inventory(_)
        ));
        lookup.clear(root.path().to_owned()).unwrap();
        assert!(lookup.clear(other_root.path().to_owned()).is_err());
        assert_eq!(
            lookup.lookup_inventory(root.path(), &roots, &targets),
            InventoryLookup::Pending
        );
        assert!(matches!(
            lookup.lookup(
                root.path(),
                "not-yet-present.mkv",
                &MediaExtractionSettings::sampled_fast_audio_index_v3(),
            ),
            FingerprintLookup::Pending
        ));
        assert!(
            rx.try_recv().is_err(),
            "clear must wait for the active read"
        );
        drop(tx);
        let (failed_root, result) = lookup.take_clear_result().unwrap();
        assert_eq!(failed_root, root.path());
        assert!(result.unwrap_err().contains("worker stopped"));
        assert!(!lookup.is_clearing());
        assert!(!lookup.in_flight);
        assert!(lookup.worker.is_none());
        assert!(lookup.take_clear_result().is_none());
    }

    #[test]
    fn inaccessible_index_reports_failure_and_can_recover_after_clear() {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("episode.mkv");
        fs::write(&media, b"media").unwrap();
        fs::create_dir(root.path().join("cache")).unwrap();
        let obstruction = root.path().join("cache/media-match");
        fs::write(&obstruction, b"index path is a file").unwrap();
        let mut lookup = GuiMediaMatchRecordLookup::default();
        assert!(matches!(
            lookup.wait_for_inventory_test(
                root.path(),
                &[root.path().to_owned()],
                &["episode.mkv".to_owned()],
            ),
            InventoryLookup::Failed(_)
        ));
        assert!(matches!(
            lookup.wait_for_test(root.path(), media.to_str().unwrap()),
            FingerprintLookup::Failed(_)
        ));
        lookup.clear(root.path().to_owned()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((failed_root, result)) = lookup.take_clear_result() {
                assert_eq!(failed_root, root.path());
                assert!(result.is_err());
                break;
            }
            assert!(Instant::now() < deadline, "clear failure must complete");
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!lookup.is_clearing());
        assert_eq!(fs::read(&obstruction).unwrap(), b"index path is a file");
        fs::remove_file(obstruction).unwrap();
        assert!(matches!(
            lookup.wait_for_test(root.path(), media.to_str().unwrap()),
            FingerprintLookup::Missing
        ));
        assert_eq!(
            lookup.lookup_inventory(root.path(), &[], &[]),
            InventoryLookup::Ready(None)
        );
    }

    #[test]
    fn non_file_and_unreadable_media_do_not_start_a_fingerprint_lookup() {
        let root = tempfile::tempdir().unwrap();
        let mut lookup = GuiMediaMatchRecordLookup::default();
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        assert!(matches!(
            lookup.lookup(root.path(), root.path().to_str().unwrap(), &settings),
            FingerprintLookup::Missing
        ));
        assert!(matches!(
            lookup.lookup(
                root.path(),
                root.path().join("missing.mkv").to_str().unwrap(),
                &settings
            ),
            FingerprintLookup::Failed(_)
        ));
        assert!(lookup.worker.is_none());
    }

    #[test]
    fn pending_fingerprint_is_distinct_from_missing_and_changed_file_invalidates_miss() {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("episode.mkv");
        fs::write(&media, b"first").unwrap();
        let path = media.to_str().unwrap();
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let mut lookup = GuiMediaMatchRecordLookup::default();
        assert!(matches!(
            lookup.lookup(root.path(), path, &settings),
            FingerprintLookup::Pending
        ));
        assert!(matches!(
            lookup.wait_for_test(root.path(), path),
            FingerprintLookup::Missing
        ));
        fs::write(&media, b"different file size").unwrap();
        assert!(matches!(
            lookup.lookup(root.path(), path, &settings),
            FingerprintLookup::Pending
        ));
        assert!(matches!(
            lookup.wait_for_test(root.path(), path),
            FingerprintLookup::Missing
        ));
    }

    #[test]
    fn invalidation_discards_an_in_flight_negative_result() {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("episode.mkv");
        fs::write(&media, b"media").unwrap();
        let path = media.to_str().unwrap();
        let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
        let (requests, rx) = mpsc::sync_channel(1);
        let (tx, replies) = mpsc::channel();
        let mut lookup = GuiMediaMatchRecordLookup {
            worker: Some(LookupWorker { requests, replies }),
            ..Default::default()
        };
        assert!(matches!(
            lookup.lookup(root.path(), path, &settings),
            FingerprintLookup::Pending
        ));
        let LookupRequest::Read(old_key) = rx.recv().unwrap() else {
            panic!("read expected")
        };
        lookup.invalidate();
        tx.send(LookupEvent::Read(LookupReply {
            key: old_key,
            result: FingerprintLookup::Missing,
            completed_at: Instant::now(),
        }))
        .unwrap();
        assert!(matches!(
            lookup.lookup(root.path(), path, &settings),
            FingerprintLookup::Pending
        ));
        let LookupRequest::Read(current_key) = rx.recv().unwrap() else {
            panic!("read expected")
        };
        tx.send(LookupEvent::Read(LookupReply {
            key: current_key,
            result: FingerprintLookup::Missing,
            completed_at: Instant::now(),
        }))
        .unwrap();
        assert!(matches!(
            lookup.lookup(root.path(), path, &settings),
            FingerprintLookup::Missing
        ));
    }

    #[test]
    fn clearing_cache_releases_retained_connection_before_removing_index() {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("episode.mkv");
        fs::write(&media, b"media").unwrap();
        let mut lookup = GuiMediaMatchRecordLookup::default();
        assert!(matches!(
            lookup.wait_for_test(root.path(), media.to_str().unwrap()),
            FingerprintLookup::Missing
        ));
        assert!(root.path().join("cache/media-match").is_dir());
        lookup.clear(root.path().to_owned()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((cleared_root, result)) = lookup.take_clear_result() {
                assert_eq!(cleared_root, root.path());
                result.unwrap();
                break;
            }
            assert!(Instant::now() < deadline, "clear did not finish");
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!lookup.is_clearing());
        assert!(!root.path().join("cache/media-match").exists());
    }
}
