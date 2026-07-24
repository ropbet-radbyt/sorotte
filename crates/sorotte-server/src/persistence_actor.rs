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
    PersistedRoomState, RoomPersistenceError, RoomPersistenceStore, StatsPersistenceError,
    StatsPersistenceStore,
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
        owner_bucket: Option<String>,
        created_at_seconds: f64,
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

#[derive(Debug, Default)]
struct RoomPersistenceDesiredState {
    highest_seen_version: u64,
    desired_effect: Option<ServerPersistenceEffect>,
    unresolved_failure_version: Option<u64>,
}

type DesiredRoomEffects = Arc<Mutex<BTreeMap<String, RoomPersistenceDesiredState>>>;

#[derive(Debug)]
pub(crate) struct RoomPersistenceService {
    worker: PersistenceWorkerService,
    desired_effects: DesiredRoomEffects,
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
        let desired_effects = Arc::new(Mutex::new(BTreeMap::new()));
        let worker_desired_effects = Arc::clone(&desired_effects);
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
                    worker_desired_effects,
                )
            },
        );
        Ok(Self {
            worker,
            desired_effects,
        })
    }

    pub(crate) fn enqueue(&self, effect: ServerPersistenceEffect) {
        let Some((room_name, version)) = room_effect_key_and_version(&effect) else {
            self.worker
                .reporter
                .failed(effect, "stats effect was routed to the room worker");
            return;
        };
        let room_name = room_name.to_owned();
        let mut states = self
            .desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = states.entry(room_name).or_default();
        if version <= state.highest_seen_version {
            drop(states);
            self.worker.reporter.ignored_stale(effect);
            return;
        }
        state.highest_seen_version = version;
        state.desired_effect = Some(effect.clone());
        state.unresolved_failure_version = None;
        drop(states);

        if !self.worker.wake() {
            self.worker
                .reporter
                .failed(effect, "persistence worker is disconnected");
        }
    }

    pub(crate) fn flush(&self) -> bool {
        self.worker.flush()
            && self
                .desired_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .all(|state| {
                    state.desired_effect.is_none() && state.unresolved_failure_version.is_none()
                })
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
    desired_effects: DesiredRoomEffects,
) {
    while let Ok(command) = commands.recv() {
        match command {
            PersistenceWorkerCommand::Apply(effect) => reporter.failed(
                effect,
                "room persistence effects must be routed through the desired-state map",
            ),
            PersistenceWorkerCommand::Wake => {
                apply_desired_room_effects(&reporter, &store, &connection, &desired_effects)
            }
            PersistenceWorkerCommand::Flush(acknowledge) => {
                apply_desired_room_effects(&reporter, &store, &connection, &desired_effects);
                let _ = acknowledge.send(());
            }
            PersistenceWorkerCommand::Shutdown => {
                apply_desired_room_effects(&reporter, &store, &connection, &desired_effects);
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

fn apply_desired_room_effects(
    reporter: &PersistenceEventReporter,
    store: &RoomPersistenceStore,
    connection: &Connection,
    desired_effects: &DesiredRoomEffects,
) {
    let effects = {
        let states = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states
            .values()
            .filter_map(|state| state.desired_effect.clone())
            .collect::<Vec<_>>()
    };

    let mut applied_any = false;
    for effect in effects {
        let Some((room_name, version)) = room_effect_key_and_version(&effect) else {
            reporter.failed(effect, "stats effect was routed to the room worker");
            continue;
        };
        let room_name = room_name.to_owned();
        let is_current = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&room_name)
            .is_some_and(|state| {
                state.highest_seen_version == version
                    && state
                        .desired_effect
                        .as_ref()
                        .and_then(room_effect_key_and_version)
                        .is_some_and(|(_, desired_version)| desired_version == version)
            });
        if !is_current {
            reporter.ignored_stale(effect);
            continue;
        }

        let transaction = match connection.unchecked_transaction() {
            Ok(transaction) => transaction,
            Err(error) => {
                reporter.failed(effect, error.to_string());
                continue;
            }
        };
        let result = match &effect {
            ServerPersistenceEffect::SaveRoom {
                room_name,
                files,
                playlist_index,
                position,
                last_activity_at_seconds,
                owner_bucket,
                created_at_seconds,
                version,
                ..
            } => store.save_room(
                &transaction,
                room_name,
                &PersistedRoomState {
                    files: files.clone(),
                    index: *playlist_index,
                    position: *position,
                    last_activity_at_seconds: *last_activity_at_seconds,
                    version: *version,
                    owner_bucket: owner_bucket.clone(),
                    created_at_seconds: *created_at_seconds,
                },
            ),
            ServerPersistenceEffect::DeleteRoom { room_name, version } => {
                store.delete_room(&transaction, room_name, *version)
            }
            ServerPersistenceEffect::RecordStatsSnapshot { .. } => unreachable!(),
        };
        let still_current = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&room_name)
            .is_some_and(|state| state.highest_seen_version == version);
        let result = match (result, still_current) {
            (Ok(()), true) => transaction.commit().map_err(|error| error.to_string()),
            (Ok(()), false) => {
                drop(transaction);
                reporter.ignored_stale(effect);
                continue;
            }
            (Err(error), _) => {
                drop(transaction);
                Err(error.to_string())
            }
        };
        match result {
            Ok(()) => {
                let is_delete = matches!(effect, ServerPersistenceEffect::DeleteRoom { .. });
                let mut states = desired_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if states
                    .get(&room_name)
                    .is_some_and(|state| state.highest_seen_version == version)
                {
                    if is_delete {
                        states.remove(&room_name);
                    } else if let Some(state) = states.get_mut(&room_name) {
                        state.desired_effect = None;
                        state.unresolved_failure_version = None;
                    }
                }
                drop(states);
                reporter.applied(effect);
                applied_any = true;
            }
            Err(error) => {
                let mut states = desired_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(state) = states.get_mut(&room_name)
                    && state.highest_seen_version == version
                {
                    state.unresolved_failure_version = Some(version);
                }
                drop(states);
                reporter.failed(effect, error);
            }
        }
    }

    if applied_any
        && desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .all(|state| {
                state.desired_effect.is_none() && state.unresolved_failure_version.is_none()
            })
    {
        reporter.recover_if_needed();
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
        Ok(()) => {
            reporter.applied(effect);
            reporter.recover_if_needed();
        }
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

    use rusqlite::{Connection, OptionalExtension};
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

    fn save_room_effect(room_name: &str, file_name: &str, version: u64) -> ServerPersistenceEffect {
        ServerPersistenceEffect::SaveRoom {
            room_name: room_name.to_owned(),
            files: vec![file_name.to_owned()],
            playlist_index: Some(0),
            position: version as f64,
            last_activity_at_seconds: version as f64,
            owner_bucket: None,
            created_at_seconds: 0.0,
            version,
        }
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
            owner_bucket: None,
            created_at_seconds: 0.0,
            version: 2,
        });
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["stale.mkv".to_owned()],
            playlist_index: Some(0),
            position: 10.0,
            last_activity_at_seconds: 10.0,
            owner_bucket: None,
            created_at_seconds: 0.0,
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
            owner_bucket: None,
            created_at_seconds: 0.0,
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
            owner_bucket: None,
            created_at_seconds: 0.0,
            version: 1,
        });
        assert!(
            !service.flush(),
            "flush must report the unresolved newest room write"
        );
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
            owner_bucket: None,
            created_at_seconds: 0.0,
            version: 1,
        });
        assert!(
            !service.flush(),
            "an ignored stale write must not hide the unresolved room failure"
        );
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
            owner_bucket: None,
            created_at_seconds: 0.0,
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
            owner_bucket: None,
            created_at_seconds: 0.0,
            version: 1,
        });
        service.enqueue(ServerPersistenceEffect::SaveRoom {
            room_name: "room".to_owned(),
            files: vec!["second.mkv".to_owned()],
            playlist_index: Some(0),
            position: 20.0,
            last_activity_at_seconds: 20.0,
            owner_bucket: None,
            created_at_seconds: 0.0,
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

    #[test]
    fn failed_newest_room_effect_is_retained_and_older_effects_cannot_report_recovery() {
        let db_path = temporary_sqlite_path("room-worker-newest-failure");
        let _ = fs::remove_file(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let (events, _) = broadcast::channel(128);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = RoomPersistenceService::start_with_queue_capacity(
            store,
            events,
            degraded_worker_count.clone(),
            1,
        )
        .expect("room persistence worker should start");
        let external = Connection::open(&db_path).expect("external sqlite connection should open");
        external
            .execute_batch(
                "CREATE TRIGGER fail_room_version_4 \
                 BEFORE INSERT ON persistent_rooms \
                 WHEN NEW.name = 'room' AND NEW.persistenceVersion = 4 \
                 BEGIN \
                     SELECT RAISE(FAIL, 'injected newest write failure'); \
                 END",
            )
            .expect("newest-version failure trigger should be installable");

        for version in 1..=4 {
            service.enqueue(save_room_effect(
                "room",
                &format!("version-{version}.mkv"),
                version,
            ));
        }
        assert!(
            !service.flush(),
            "flush must expose the retained version-4 failure"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        let durable_before_stale: Option<(String, i64)> = external
            .query_row(
                "SELECT playlist, persistenceVersion FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("persisted room should be queryable");

        service.enqueue(save_room_effect("room", "stale-2.mkv", 2));
        service.enqueue(save_room_effect("room", "stale-3.mkv", 3));
        service.enqueue(save_room_effect("other", "other.mkv", 1));
        assert!(
            !service.flush(),
            "older and unrelated successes must not hide the newest failure"
        );
        let durable_after_stale: Option<(String, i64)> = external
            .query_row(
                "SELECT playlist, persistenceVersion FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("persisted room should remain queryable");
        assert_eq!(durable_after_stale, durable_before_stale);
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        let unresolved_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
        assert!(
            !unresolved_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
        );

        external
            .execute("DROP TRIGGER fail_room_version_4", [])
            .expect("failure trigger should be removable");
        assert!(
            service.flush(),
            "retained newest desired state should retry successfully"
        );
        let durable: (String, i64) = external
            .query_row(
                "SELECT playlist, persistenceVersion FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("newest room state should persist");
        assert_eq!(durable, ("version-4.mkv".to_owned(), 4));
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);

        drop(service);
        drop(external);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn failed_newest_delete_cannot_be_superseded_by_an_older_save() {
        let db_path = temporary_sqlite_path("room-worker-delete-failure");
        let _ = fs::remove_file(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let (events, _) = broadcast::channel(64);
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = RoomPersistenceService::start_with_queue_capacity(
            store,
            events,
            degraded_worker_count.clone(),
            1,
        )
        .expect("room persistence worker should start");
        let external = Connection::open(&db_path).expect("external sqlite connection should open");

        service.enqueue(save_room_effect("room", "baseline.mkv", 1));
        assert!(service.flush(), "baseline should persist");
        external
            .execute_batch(
                "CREATE TRIGGER fail_room_delete \
                 BEFORE DELETE ON persistent_rooms \
                 WHEN OLD.name = 'room' \
                 BEGIN \
                     SELECT RAISE(FAIL, 'injected delete failure'); \
                 END",
            )
            .expect("delete failure trigger should be installable");
        service.enqueue(ServerPersistenceEffect::DeleteRoom {
            room_name: "room".to_owned(),
            version: 4,
        });
        service.enqueue(save_room_effect("room", "stale-2.mkv", 2));
        service.enqueue(save_room_effect("room", "stale-3.mkv", 3));
        assert!(
            !service.flush(),
            "failed newest delete must remain unresolved"
        );
        let durable: (String, i64) = external
            .query_row(
                "SELECT playlist, persistenceVersion FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("baseline room should remain until delete retry");
        assert_eq!(durable, ("baseline.mkv".to_owned(), 1));
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);

        external
            .execute("DROP TRIGGER fail_room_delete", [])
            .expect("delete failure trigger should be removable");
        assert!(service.flush(), "newest delete should retry");
        let count: i64 = external
            .query_row(
                "SELECT COUNT(*) FROM persistent_rooms WHERE name = 'room'",
                [],
                |row| row.get(0),
            )
            .expect("room count should be queryable");
        assert_eq!(count, 0);
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);

        drop(service);
        drop(external);
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }
}
