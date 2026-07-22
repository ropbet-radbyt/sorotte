use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
};

use rusqlite::Connection;
use tokio::sync::broadcast;

use crate::{
    RoomPersistenceError, RoomPersistenceStore, StatsPersistenceError, StatsPersistenceStore,
};

const PERSISTENCE_COMMAND_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerPersistenceWorkerKind {
    Rooms,
    Stats,
}

#[derive(Clone, PartialEq)]
pub enum ServerPersistenceEffect {
    SaveRoom {
        room_name: String,
        files: Vec<String>,
        playlist_index: Option<i64>,
        position: f64,
        last_activity_at_seconds: f64,
        version: u64,
    },
    DeleteRoom {
        room_name: String,
        version: u64,
    },
    RecordStatsSnapshot {
        snapshot_time: i64,
        versions: Vec<String>,
    },
}

impl std::fmt::Debug for ServerPersistenceEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SaveRoom {
                files,
                playlist_index,
                position,
                version,
                ..
            } => formatter
                .debug_struct("SaveRoom")
                .field("room_name", &sorotte_secret::REDACTED_SECRET)
                .field("files_count", &files.len())
                .field("playlist_index", playlist_index)
                .field("position", position)
                .field("version", version)
                .finish(),
            Self::DeleteRoom { version, .. } => formatter
                .debug_struct("DeleteRoom")
                .field("room_name", &sorotte_secret::REDACTED_SECRET)
                .field("version", version)
                .finish(),
            Self::RecordStatsSnapshot {
                snapshot_time,
                versions,
            } => formatter
                .debug_struct("RecordStatsSnapshot")
                .field("snapshot_time", snapshot_time)
                .field("versions_count", &versions.len())
                .finish(),
        }
    }
}

/// Persistence failures put only the affected worker into degraded mode. The
/// ordered server model and network continue running; later room snapshots can
/// compensate for failed writes, while later stats snapshots continue normally.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerPersistenceEvent {
    Applied {
        worker: ServerPersistenceWorkerKind,
        effect: ServerPersistenceEffect,
    },
    IgnoredStale {
        worker: ServerPersistenceWorkerKind,
        effect: ServerPersistenceEffect,
    },
    Failed {
        worker: ServerPersistenceWorkerKind,
        effect: ServerPersistenceEffect,
        error: String,
    },
    Degraded {
        worker: ServerPersistenceWorkerKind,
    },
    Recovered {
        worker: ServerPersistenceWorkerKind,
    },
}

enum PersistenceWorkerCommand {
    Apply(ServerPersistenceEffect),
    Wake,
    Flush(mpsc::Sender<()>),
    Shutdown,
}

enum PersistenceEnqueueError {
    Full(ServerPersistenceEffect),
    Disconnected(ServerPersistenceEffect),
}

#[derive(Clone)]
struct PersistenceEventReporter {
    worker: ServerPersistenceWorkerKind,
    events: broadcast::Sender<ServerPersistenceEvent>,
    worker_degraded: Arc<AtomicBool>,
    degraded_worker_count: Arc<AtomicUsize>,
}

impl PersistenceEventReporter {
    fn applied(&self, effect: ServerPersistenceEffect) {
        let _ = self.events.send(ServerPersistenceEvent::Applied {
            worker: self.worker,
            effect,
        });
        self.recover_if_needed();
    }

    fn ignored_stale(&self, effect: ServerPersistenceEffect) {
        let _ = self.events.send(ServerPersistenceEvent::IgnoredStale {
            worker: self.worker,
            effect,
        });
    }

    fn failed(&self, effect: ServerPersistenceEffect, error: impl Into<String>) {
        let error = error.into();
        let _ = self.events.send(ServerPersistenceEvent::Failed {
            worker: self.worker,
            effect,
            error: error.clone(),
        });
        if !self.worker_degraded.swap(true, Ordering::AcqRel) {
            self.degraded_worker_count.fetch_add(1, Ordering::AcqRel);
            let _ = self.events.send(ServerPersistenceEvent::Degraded {
                worker: self.worker,
            });
        }
        eprintln!(
            "Sorotte server {:?} persistence entered degraded mode: {error}",
            self.worker
        );
    }

    fn recover_if_needed(&self) {
        if self.worker_degraded.swap(false, Ordering::AcqRel) {
            let _ = self.degraded_worker_count.fetch_update(
                Ordering::AcqRel,
                Ordering::Acquire,
                |count| Some(count.saturating_sub(1)),
            );
            let _ = self.events.send(ServerPersistenceEvent::Recovered {
                worker: self.worker,
            });
        }
    }
}

struct PersistenceWorkerService {
    worker: ServerPersistenceWorkerKind,
    commands: SyncSender<PersistenceWorkerCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
    reporter: PersistenceEventReporter,
}

impl std::fmt::Debug for PersistenceWorkerService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersistenceWorkerService")
            .field("worker", &self.worker)
            .field(
                "degraded",
                &self.reporter.worker_degraded.load(Ordering::Acquire),
            )
            .finish_non_exhaustive()
    }
}

impl PersistenceWorkerService {
    fn spawn(
        worker: ServerPersistenceWorkerKind,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
        run: impl FnOnce(Receiver<PersistenceWorkerCommand>, PersistenceEventReporter) + Send + 'static,
    ) -> Self {
        Self::spawn_with_capacity(
            worker,
            events,
            degraded_worker_count,
            PERSISTENCE_COMMAND_QUEUE_CAPACITY,
            run,
        )
    }

    fn spawn_with_capacity(
        worker: ServerPersistenceWorkerKind,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
        queue_capacity: usize,
        run: impl FnOnce(Receiver<PersistenceWorkerCommand>, PersistenceEventReporter) + Send + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::sync_channel::<PersistenceWorkerCommand>(queue_capacity);
        let reporter = PersistenceEventReporter {
            worker,
            events,
            worker_degraded: Arc::new(AtomicBool::new(false)),
            degraded_worker_count,
        };
        let worker_reporter = reporter.clone();
        let join_handle = thread::Builder::new()
            .name(format!("sorotte-persistence-{worker:?}"))
            .spawn(move || run(receiver, worker_reporter))
            .expect("persistence worker thread should spawn");
        Self {
            worker,
            commands,
            join_handle: Some(join_handle),
            reporter,
        }
    }

    fn try_enqueue(&self, effect: ServerPersistenceEffect) -> Result<(), PersistenceEnqueueError> {
        match self
            .commands
            .try_send(PersistenceWorkerCommand::Apply(effect))
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(PersistenceWorkerCommand::Apply(effect))) => {
                Err(PersistenceEnqueueError::Full(effect))
            }
            Err(TrySendError::Disconnected(PersistenceWorkerCommand::Apply(effect))) => {
                Err(PersistenceEnqueueError::Disconnected(effect))
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                unreachable!("enqueue only sends apply commands")
            }
        }
    }

    fn enqueue(&self, effect: ServerPersistenceEffect) {
        match self.try_enqueue(effect) {
            Ok(()) => {}
            Err(PersistenceEnqueueError::Full(effect)) => self
                .reporter
                .failed(effect, "persistence command queue is full"),
            Err(PersistenceEnqueueError::Disconnected(effect)) => self
                .reporter
                .failed(effect, "persistence worker is disconnected"),
        }
    }

    fn wake(&self) -> bool {
        match self.commands.try_send(PersistenceWorkerCommand::Wake) {
            Ok(()) | Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    fn flush(&self) -> bool {
        let (acknowledge, acknowledgement) = mpsc::channel();
        self.commands
            .send(PersistenceWorkerCommand::Flush(acknowledge))
            .is_ok()
            && acknowledgement.recv().is_ok()
    }
}

impl Drop for PersistenceWorkerService {
    fn drop(&mut self) {
        let _ = self.commands.send(PersistenceWorkerCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

type CoalescedRoomEffects = Arc<Mutex<BTreeMap<String, ServerPersistenceEffect>>>;

#[derive(Debug)]
pub(crate) struct RoomPersistenceService {
    worker: PersistenceWorkerService,
    coalesced_effects: CoalescedRoomEffects,
}

impl RoomPersistenceService {
    pub(crate) fn start(
        store: RoomPersistenceStore,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
    ) -> Result<Self, RoomPersistenceError> {
        Self::start_with_queue_capacity(
            store,
            events,
            degraded_worker_count,
            PERSISTENCE_COMMAND_QUEUE_CAPACITY,
        )
    }

    fn start_with_queue_capacity(
        store: RoomPersistenceStore,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
        queue_capacity: usize,
    ) -> Result<Self, RoomPersistenceError> {
        let connection = store.connection("connect persistence worker")?;
        let coalesced_effects = Arc::new(Mutex::new(BTreeMap::new()));
        let worker_coalesced_effects = Arc::clone(&coalesced_effects);
        let worker = PersistenceWorkerService::spawn_with_capacity(
            ServerPersistenceWorkerKind::Rooms,
            events,
            degraded_worker_count,
            queue_capacity,
            move |commands, reporter| {
                run_room_worker(
                    commands,
                    reporter,
                    store,
                    connection,
                    worker_coalesced_effects,
                )
            },
        );
        Ok(Self {
            worker,
            coalesced_effects,
        })
    }

    pub(crate) fn enqueue(&self, effect: ServerPersistenceEffect) {
        match self.worker.try_enqueue(effect) {
            Ok(()) => {}
            Err(PersistenceEnqueueError::Full(effect)) => {
                self.coalesce(effect);
            }
            Err(PersistenceEnqueueError::Disconnected(effect)) => self
                .worker
                .reporter
                .failed(effect, "persistence worker is disconnected"),
        }
    }

    pub(crate) fn flush(&self) -> bool {
        self.worker.flush()
    }

    fn coalesce(&self, effect: ServerPersistenceEffect) {
        let Some((room_name, version)) = room_effect_key_and_version(&effect) else {
            self.worker
                .reporter
                .failed(effect, "stats effect was routed to the room worker");
            return;
        };
        let room_name = room_name.to_owned();
        let mut effects = self
            .coalesced_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let should_replace = effects
            .get(&room_name)
            .and_then(room_effect_key_and_version)
            .is_none_or(|(_, pending_version)| pending_version < version);
        if should_replace {
            effects.insert(room_name, effect);
        } else {
            drop(effects);
            self.worker.reporter.ignored_stale(effect);
            return;
        }
        drop(effects);

        if !self.worker.wake() {
            let mut effects = self
                .coalesced_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for effect in std::mem::take(&mut *effects).into_values() {
                self.worker
                    .reporter
                    .failed(effect, "persistence worker is disconnected");
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct StatsPersistenceService(PersistenceWorkerService);

impl StatsPersistenceService {
    pub(crate) fn start(
        store: StatsPersistenceStore,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
    ) -> Result<Self, StatsPersistenceError> {
        let connection = store.connection("connect persistence worker")?;
        Ok(Self(PersistenceWorkerService::spawn(
            ServerPersistenceWorkerKind::Stats,
            events,
            degraded_worker_count,
            move |commands, reporter| run_stats_worker(commands, reporter, store, connection),
        )))
    }

    pub(crate) fn enqueue(&self, effect: ServerPersistenceEffect) {
        self.0.enqueue(effect);
    }

    pub(crate) fn flush(&self) -> bool {
        self.0.flush()
    }
}

fn run_room_worker(
    commands: Receiver<PersistenceWorkerCommand>,
    reporter: PersistenceEventReporter,
    store: RoomPersistenceStore,
    connection: Connection,
    coalesced_effects: CoalescedRoomEffects,
) {
    let mut latest_versions = BTreeMap::<String, u64>::new();
    while let Ok(command) = commands.recv() {
        match command {
            PersistenceWorkerCommand::Apply(effect) => {
                apply_room_effect(&reporter, &store, &connection, &mut latest_versions, effect);
                drain_coalesced_room_effects(
                    &reporter,
                    &store,
                    &connection,
                    &mut latest_versions,
                    &coalesced_effects,
                );
            }
            PersistenceWorkerCommand::Wake => drain_coalesced_room_effects(
                &reporter,
                &store,
                &connection,
                &mut latest_versions,
                &coalesced_effects,
            ),
            PersistenceWorkerCommand::Flush(acknowledge) => {
                drain_coalesced_room_effects(
                    &reporter,
                    &store,
                    &connection,
                    &mut latest_versions,
                    &coalesced_effects,
                );
                let _ = acknowledge.send(());
            }
            PersistenceWorkerCommand::Shutdown => {
                drain_coalesced_room_effects(
                    &reporter,
                    &store,
                    &connection,
                    &mut latest_versions,
                    &coalesced_effects,
                );
                break;
            }
        }
    }
}

fn room_effect_key_and_version(effect: &ServerPersistenceEffect) -> Option<(&str, u64)> {
    match effect {
        ServerPersistenceEffect::SaveRoom {
            room_name, version, ..
        }
        | ServerPersistenceEffect::DeleteRoom { room_name, version } => {
            Some((room_name.as_str(), *version))
        }
        ServerPersistenceEffect::RecordStatsSnapshot { .. } => None,
    }
}

fn drain_coalesced_room_effects(
    reporter: &PersistenceEventReporter,
    store: &RoomPersistenceStore,
    connection: &Connection,
    latest_versions: &mut BTreeMap<String, u64>,
    coalesced_effects: &CoalescedRoomEffects,
) {
    loop {
        let effects = {
            let mut effects = coalesced_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *effects)
        };
        if effects.is_empty() {
            return;
        }
        for effect in effects.into_values() {
            apply_room_effect(reporter, store, connection, latest_versions, effect);
        }
    }
}

fn apply_room_effect(
    reporter: &PersistenceEventReporter,
    store: &RoomPersistenceStore,
    connection: &Connection,
    latest_versions: &mut BTreeMap<String, u64>,
    effect: ServerPersistenceEffect,
) {
    let (room_name, version) = match &effect {
        ServerPersistenceEffect::SaveRoom {
            room_name, version, ..
        }
        | ServerPersistenceEffect::DeleteRoom { room_name, version } => (room_name, *version),
        ServerPersistenceEffect::RecordStatsSnapshot { .. } => {
            reporter.failed(effect, "stats effect was routed to the room worker");
            return;
        }
    };
    if latest_versions
        .get(room_name)
        .is_some_and(|latest| *latest >= version)
    {
        reporter.ignored_stale(effect);
        return;
    }

    let result = match &effect {
        ServerPersistenceEffect::SaveRoom {
            room_name,
            files,
            playlist_index,
            position,
            last_activity_at_seconds,
            ..
        } => store.save_room(
            connection,
            room_name,
            files,
            *playlist_index,
            *position,
            *last_activity_at_seconds,
        ),
        ServerPersistenceEffect::DeleteRoom { room_name, .. } => {
            store.delete_room(connection, room_name)
        }
        ServerPersistenceEffect::RecordStatsSnapshot { .. } => unreachable!(),
    };
    match result {
        Ok(()) => {
            latest_versions.insert(room_name.clone(), version);
            reporter.applied(effect);
        }
        Err(error) => reporter.failed(effect, error.to_string()),
    }
}

fn run_stats_worker(
    commands: Receiver<PersistenceWorkerCommand>,
    reporter: PersistenceEventReporter,
    store: StatsPersistenceStore,
    mut connection: Connection,
) {
    while let Ok(command) = commands.recv() {
        match command {
            PersistenceWorkerCommand::Apply(effect) => {
                apply_stats_effect(&reporter, &store, &mut connection, effect);
            }
            PersistenceWorkerCommand::Wake => {}
            PersistenceWorkerCommand::Flush(acknowledge) => {
                let _ = acknowledge.send(());
            }
            PersistenceWorkerCommand::Shutdown => break,
        }
    }
}

fn apply_stats_effect(
    reporter: &PersistenceEventReporter,
    store: &StatsPersistenceStore,
    connection: &mut Connection,
    effect: ServerPersistenceEffect,
) {
    let result = match &effect {
        ServerPersistenceEffect::RecordStatsSnapshot {
            snapshot_time,
            versions,
        } => store.add_version_logs(connection, *snapshot_time, versions),
        ServerPersistenceEffect::SaveRoom { .. } | ServerPersistenceEffect::DeleteRoom { .. } => {
            reporter.failed(effect, "room effect was routed to the stats worker");
            return;
        }
    };
    match result {
        Ok(()) => reporter.applied(effect),
        Err(error) => reporter.failed(effect, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use rusqlite::Connection;
    use tokio::sync::broadcast;

    use super::{
        RoomPersistenceService, ServerPersistenceEffect, ServerPersistenceEvent,
        StatsPersistenceService,
    };
    use crate::{RoomPersistenceStore, StatsPersistenceStore};

    fn temporary_sqlite_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sorotte-{name}-{}-{unique}.sqlite3",
            std::process::id()
        ))
    }

    #[test]
    fn stats_worker_surfaces_degraded_and_recovered_events() {
        let db_path = temporary_sqlite_path("stats-worker-recovery");
        let _ = fs::remove_file(&db_path);
        let store = StatsPersistenceStore::open(&db_path)
            .expect("stats persistence schema should initialize");
        let (events, _) = broadcast::channel(16);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = StatsPersistenceService::start(store, events, degraded_worker_count.clone())
            .expect("stats persistence worker should start");
        let external = Connection::open(&db_path).expect("external sqlite connection should open");
        external
            .execute("DROP TABLE clients_snapshots", [])
            .expect("stats table should be removable for failure injection");

        service.enqueue(ServerPersistenceEffect::RecordStatsSnapshot {
            snapshot_time: 1,
            versions: vec!["1.7.0".to_owned()],
        });
        assert!(
            service.flush(),
            "failed effect should still be acknowledged"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        let failed_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(
            failed_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Failed { .. }))
        );
        assert!(
            failed_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Degraded { .. }))
        );

        external
            .execute(
                "CREATE TABLE clients_snapshots (snapshot_time INTEGER, version STRING)",
                [],
            )
            .expect("stats table should be restorable");
        service.enqueue(ServerPersistenceEffect::RecordStatsSnapshot {
            snapshot_time: 2,
            versions: vec!["1.7.1".to_owned(), "1.7.2".to_owned()],
        });
        assert!(service.flush(), "recovery effect should be acknowledged");
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);
        let recovery_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(
            recovery_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Applied { .. }))
        );
        assert!(
            recovery_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
        );

        drop(service);
        drop(external);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn room_worker_ignores_stale_versioned_snapshots() {
        let db_path = temporary_sqlite_path("room-worker-versioning");
        let _ = fs::remove_file(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let (events, _) = broadcast::channel(16);
        let mut event_rx = events.subscribe();
        let service = RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
            .expect("room persistence worker should start");

        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["new.mkv".to_owned()],
            playlist_index: Some(0),
            position: 20.0,
            last_activity_at_seconds: 20.0,
            version: 2,
        });
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["stale.mkv".to_owned()],
            playlist_index: Some(0),
            position: 10.0,
            last_activity_at_seconds: 10.0,
            version: 1,
        });
        assert!(service.flush(), "room effects should be acknowledged");

        let connection = Connection::open(&db_path).expect("sqlite db should be inspectable");
        let playlist: String = connection
            .query_row(
                "SELECT playlist FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| row.get(0),
            )
            .expect("persisted room should exist");
        assert_eq!(playlist, "new.mkv");
        let worker_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(worker_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::IgnoredStale {
                effect: ServerPersistenceEffect::SaveRoom { version: 1, .. },
                ..
            }
        )));

        drop(service);
        drop(connection);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn ignored_stale_room_effect_does_not_report_database_recovery() {
        let db_path = temporary_sqlite_path("room-worker-stale-health");
        let _ = fs::remove_file(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let (events, _) = broadcast::channel(32);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = RoomPersistenceService::start(store, events, degraded_worker_count.clone())
            .expect("room persistence worker should start");
        let external = Connection::open(&db_path).expect("external sqlite connection should open");

        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room-a".to_owned(),
            files: vec!["new.mkv".to_owned()],
            playlist_index: Some(0),
            position: 20.0,
            last_activity_at_seconds: 20.0,
            version: 2,
        });
        assert!(
            service.flush(),
            "baseline room write should be acknowledged"
        );
        let _: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();

        external
            .execute_batch(
                "CREATE TRIGGER fail_room_b \
                 BEFORE INSERT ON persistent_rooms \
                 WHEN NEW.name = 'room-b' \
                 BEGIN \
                     SELECT RAISE(FAIL, 'injected room-b write failure'); \
                 END",
            )
            .expect("room-b failure trigger should be installable");
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room-b".to_owned(),
            files: vec!["blocked.mkv".to_owned()],
            playlist_index: Some(0),
            position: 30.0,
            last_activity_at_seconds: 30.0,
            version: 1,
        });
        assert!(service.flush(), "failed room write should be acknowledged");
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        let failure_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(failure_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Failed {
                effect: ServerPersistenceEffect::SaveRoom { room_name, .. },
                ..
            } if room_name == "room-b"
        )));
        assert!(
            failure_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Degraded { .. }))
        );

        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room-a".to_owned(),
            files: vec!["stale.mkv".to_owned()],
            playlist_index: Some(0),
            position: 10.0,
            last_activity_at_seconds: 10.0,
            version: 1,
        });
        assert!(service.flush(), "stale room effect should be acknowledged");
        assert_eq!(
            degraded_worker_count.load(Ordering::Acquire),
            1,
            "ignoring stale in-memory work does not prove database recovery"
        );
        let stale_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(stale_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::IgnoredStale {
                effect: ServerPersistenceEffect::SaveRoom { room_name, .. },
                ..
            } if room_name == "room-a"
        )));
        assert!(
            !stale_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Recovered { .. })),
            "a stale effect must not report persistence recovery"
        );

        external
            .execute("DROP TRIGGER fail_room_b", [])
            .expect("room-b failure trigger should be removable");
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room-b".to_owned(),
            files: vec!["recovered.mkv".to_owned()],
            playlist_index: Some(0),
            position: 40.0,
            last_activity_at_seconds: 40.0,
            version: 2,
        });
        assert!(
            service.flush(),
            "successful recovery write should be acknowledged"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);
        let recovery_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(recovery_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { room_name, .. },
                ..
            } if room_name == "room-b"
        )));
        assert!(
            recovery_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
        );

        drop(service);
        drop(external);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn room_worker_flushes_latest_mutation_after_queue_saturation() {
        let db_path = temporary_sqlite_path("room-worker-overflow");
        let _ = fs::remove_file(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let (events, _) = broadcast::channel(16);
        let service = RoomPersistenceService::start_with_queue_capacity(
            store,
            events,
            Arc::new(AtomicUsize::new(0)),
            1,
        )
        .expect("room persistence worker should start");
        let external = Connection::open(&db_path).expect("external sqlite connection should open");
        external
            .execute_batch("BEGIN EXCLUSIVE")
            .expect("external connection should lock sqlite writes");

        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["first.mkv".to_owned()],
            playlist_index: Some(0),
            position: 10.0,
            last_activity_at_seconds: 10.0,
            version: 1,
        });
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["second.mkv".to_owned()],
            playlist_index: Some(0),
            position: 20.0,
            last_activity_at_seconds: 20.0,
            version: 2,
        });
        service.enqueue(ServerPersistenceEffect::DeleteRoom {
            room_name: "room".to_owned(),
            version: 3,
        });

        external
            .execute_batch("ROLLBACK")
            .expect("external sqlite lock should release");
        assert!(
            service.flush(),
            "flush should include the coalesced room mutation"
        );

        let persisted_count: i64 = external
            .query_row(
                "SELECT COUNT(*) FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| row.get(0),
            )
            .expect("persisted rooms should be queryable");
        assert_eq!(persisted_count, 0, "the latest delete must not be dropped");

        drop(service);
        drop(external);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }
}
