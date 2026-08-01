use std::{
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

mod persistence_arbitration;
#[cfg(test)]
mod persistence_arbitration_tests;

use persistence_arbitration::{
    DesiredRoomEffects, RoomEffectEnqueueDisposition, RoomPersistenceArbitration,
    room_effect_key_and_version,
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

#[cfg(test)]
type BeforeRoomTransactionHook =
    Arc<dyn Fn(&Connection, &ServerPersistenceEffect) + Send + Sync + 'static>;

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum RoomWorkerTestCheckpoint {
    BeforeScan,
    BeforeArbitration { room_name: String, version: u64 },
    AfterArbitration { room_name: String, version: u64 },
}

#[derive(Clone, Default)]
struct RoomPersistenceWorkerHooks {
    #[cfg(test)]
    on_checkpoint: Option<Arc<dyn Fn(RoomWorkerTestCheckpoint) + Send + Sync + 'static>>,
    #[cfg(test)]
    transaction_entries: Option<Arc<Mutex<Vec<ServerPersistenceEffect>>>>,
    #[cfg(test)]
    before_transaction: Option<BeforeRoomTransactionHook>,
}

impl RoomPersistenceWorkerHooks {
    fn before_scan(&self) {
        #[cfg(test)]
        if let Some(on_checkpoint) = self.on_checkpoint.as_ref() {
            on_checkpoint(RoomWorkerTestCheckpoint::BeforeScan);
        }
    }

    fn before_arbitration(&self, _effect: &ServerPersistenceEffect) {
        #[cfg(test)]
        if let (Some(on_checkpoint), Some((room_name, version))) = (
            self.on_checkpoint.as_ref(),
            room_effect_key_and_version(_effect),
        ) {
            on_checkpoint(RoomWorkerTestCheckpoint::BeforeArbitration {
                room_name: room_name.to_owned(),
                version,
            });
        }
    }

    fn after_arbitration(&self, _effect: &ServerPersistenceEffect) {
        #[cfg(test)]
        if let (Some(on_checkpoint), Some((room_name, version))) = (
            self.on_checkpoint.as_ref(),
            room_effect_key_and_version(_effect),
        ) {
            on_checkpoint(RoomWorkerTestCheckpoint::AfterArbitration {
                room_name: room_name.to_owned(),
                version,
            });
        }
    }

    fn record_transaction_entry(&self, _effect: &ServerPersistenceEffect) {
        #[cfg(test)]
        if let Some(entries) = self.transaction_entries.as_ref() {
            entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(_effect.clone());
        }
    }

    #[cfg(test)]
    fn before_transaction(&self, connection: &Connection, effect: &ServerPersistenceEffect) {
        if let Some(before_transaction) = self.before_transaction.as_ref() {
            before_transaction(connection, effect);
        }
    }
}

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
        Self::start_with_queue_capacity_and_hooks(
            store,
            events,
            degraded_worker_count,
            queue_capacity,
            RoomPersistenceWorkerHooks::default(),
        )
    }

    fn start_with_queue_capacity_and_hooks(
        store: RoomPersistenceStore,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
        queue_capacity: usize,
        hooks: RoomPersistenceWorkerHooks,
    ) -> Result<Self, RoomPersistenceError> {
        let connection = store.connection("connect persistence worker")?;
        let desired_effects = Arc::new(Mutex::new(RoomPersistenceArbitration::default()));
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
                    hooks,
                )
            },
        );
        Ok(Self {
            worker,
            desired_effects,
        })
    }

    pub(crate) fn enqueue(&self, effect: ServerPersistenceEffect) {
        let mut arbitration = self
            .desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match arbitration.enqueue(effect.clone()) {
            RoomEffectEnqueueDisposition::Accepted => {}
            RoomEffectEnqueueDisposition::IgnoredStale => {
                drop(arbitration);
                self.worker.reporter.ignored_stale(effect);
                return;
            }
            RoomEffectEnqueueDisposition::NotRoomEffect => {
                drop(arbitration);
                self.worker
                    .reporter
                    .failed(effect, "stats effect was routed to the room worker");
                return;
            }
        }
        drop(arbitration);

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
                .is_settled()
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

    #[cfg(test)]
    fn start_with_queue_capacity_and_start_hook(
        store: StatsPersistenceStore,
        events: broadcast::Sender<ServerPersistenceEvent>,
        degraded_worker_count: Arc<AtomicUsize>,
        queue_capacity: usize,
        start_hook: impl FnOnce() + Send + 'static,
    ) -> Result<Self, StatsPersistenceError> {
        let connection = store.connection("connect persistence worker")?;
        Ok(Self(PersistenceWorkerService::spawn_with_capacity(
            ServerPersistenceWorkerKind::Stats,
            events,
            degraded_worker_count,
            queue_capacity,
            move |commands, reporter| {
                start_hook();
                run_stats_worker(commands, reporter, store, connection);
            },
        )))
    }
}

fn run_room_worker(
    commands: Receiver<PersistenceWorkerCommand>,
    reporter: PersistenceEventReporter,
    store: RoomPersistenceStore,
    connection: Connection,
    desired_effects: DesiredRoomEffects,
    hooks: RoomPersistenceWorkerHooks,
) {
    while let Ok(command) = commands.recv() {
        match command {
            PersistenceWorkerCommand::Apply(effect) => reporter.failed(
                effect,
                "room persistence effects must be routed through the desired-state map",
            ),
            PersistenceWorkerCommand::Wake => {
                apply_desired_room_effects(&reporter, &store, &connection, &desired_effects, &hooks)
            }
            PersistenceWorkerCommand::Flush(acknowledge) => {
                apply_desired_room_effects(
                    &reporter,
                    &store,
                    &connection,
                    &desired_effects,
                    &hooks,
                );
                let _ = acknowledge.send(());
            }
            PersistenceWorkerCommand::Shutdown => {
                apply_desired_room_effects(
                    &reporter,
                    &store,
                    &connection,
                    &desired_effects,
                    &hooks,
                );
                break;
            }
        }
    }
}

fn apply_desired_room_effects(
    reporter: &PersistenceEventReporter,
    store: &RoomPersistenceStore,
    connection: &Connection,
    desired_effects: &DesiredRoomEffects,
    hooks: &RoomPersistenceWorkerHooks,
) {
    hooks.before_scan();
    let effects = {
        let arbitration = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        arbitration.desired_effects()
    };

    let mut applied_any = false;
    for effect in effects {
        let Some((room_name, version)) = room_effect_key_and_version(&effect) else {
            reporter.failed(effect, "stats effect was routed to the room worker");
            continue;
        };
        let room_name = room_name.to_owned();
        hooks.before_arbitration(&effect);
        let is_current = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_effect_current(&effect);
        if !is_current {
            reporter.ignored_stale(effect);
            continue;
        }

        hooks.after_arbitration(&effect);
        hooks.record_transaction_entry(&effect);
        #[cfg(test)]
        hooks.before_transaction(connection, &effect);
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
        #[cfg(test)]
        if result.is_ok() {
            crate::persistence::test_crash::exit_if_armed(
                crate::persistence::test_crash::ROOM_EFFECT_AFTER_WRITE,
            );
        }
        let still_current = desired_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_version_current(&room_name, version);
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
                #[cfg(test)]
                crate::persistence::test_crash::exit_if_armed(
                    crate::persistence::test_crash::ROOM_EFFECT_AFTER_COMMIT,
                );
                let is_delete = matches!(effect, ServerPersistenceEffect::DeleteRoom { .. });
                let mut arbitration = desired_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                arbitration.mark_applied(&room_name, version, is_delete);
                drop(arbitration);
                reporter.applied(effect);
                applied_any = true;
            }
            Err(error) => {
                let mut arbitration = desired_effects
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                arbitration.mark_failed(&room_name, version);
                drop(arbitration);
                reporter.failed(effect, error);
            }
        }
    }

    if desired_effects
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .should_report_recovery(applied_any)
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
            #[cfg(test)]
            crate::persistence::test_crash::exit_if_armed(
                crate::persistence::test_crash::STATS_AFTER_COMMIT,
            );
            reporter.applied(effect);
            reporter.recover_if_needed();
        }
        Err(error) => reporter.failed(effect, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, VecDeque},
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
    use tokio::sync::broadcast;

    use super::{
        RoomPersistenceService, RoomPersistenceWorkerHooks, RoomWorkerTestCheckpoint,
        ServerPersistenceEffect, ServerPersistenceEvent, StatsPersistenceService,
        room_effect_key_and_version,
    };
    use crate::{
        PersistedRoomState, RoomPersistenceError, RoomPersistenceStore, StatsPersistenceStore,
        persistence::test_crash,
    };

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

    const SCHEDULE_WATCHDOG: Duration = Duration::from_secs(5);

    struct RoomWorkerCheckpointController {
        expected: Arc<Mutex<VecDeque<RoomWorkerTestCheckpoint>>>,
        reached: mpsc::Receiver<RoomWorkerTestCheckpoint>,
        release: mpsc::SyncSender<()>,
    }

    impl RoomWorkerCheckpointController {
        fn arm(&self, checkpoint: RoomWorkerTestCheckpoint) {
            self.expected
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push_back(checkpoint);
        }

        fn wait_for(&self, expected: &RoomWorkerTestCheckpoint) {
            let actual = self
                .reached
                .recv_timeout(SCHEDULE_WATCHDOG)
                .unwrap_or_else(|error| {
                    panic!("persistence worker did not reach {expected:?}: {error}")
                });
            assert_eq!(
                &actual, expected,
                "persistence worker reached the wrong controlled checkpoint"
            );
        }

        fn release(&self) {
            self.release
                .send(())
                .expect("controlled persistence worker should still be waiting");
        }
    }

    fn controlled_room_worker_hooks() -> (
        RoomPersistenceWorkerHooks,
        RoomWorkerCheckpointController,
        Arc<Mutex<Vec<ServerPersistenceEffect>>>,
    ) {
        let expected = Arc::new(Mutex::new(VecDeque::new()));
        let hook_expected = Arc::clone(&expected);
        let (reached_tx, reached_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release_rx = Arc::new(Mutex::new(release_rx));
        let hook_release_rx = Arc::clone(&release_rx);
        let transaction_entries = Arc::new(Mutex::new(Vec::new()));
        let hooks = RoomPersistenceWorkerHooks {
            on_checkpoint: Some(Arc::new(move |checkpoint| {
                let should_block = {
                    let mut expected = hook_expected
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if expected.front() == Some(&checkpoint) {
                        expected.pop_front();
                        true
                    } else {
                        false
                    }
                };
                if !should_block {
                    return;
                }
                reached_tx
                    .send(checkpoint)
                    .expect("checkpoint controller should still be listening");
                hook_release_rx
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recv_timeout(SCHEDULE_WATCHDOG)
                    .expect("checkpoint controller should release the persistence worker");
            })),
            transaction_entries: Some(Arc::clone(&transaction_entries)),
            ..RoomPersistenceWorkerHooks::default()
        };
        (
            hooks,
            RoomWorkerCheckpointController {
                expected,
                reached: reached_rx,
                release: release_tx,
            },
            transaction_entries,
        )
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ScheduledRoomMutation {
        Save { version: u64 },
        Delete { version: u64 },
    }

    impl ScheduledRoomMutation {
        fn version(self) -> u64 {
            match self {
                Self::Save { version } | Self::Delete { version } => version,
            }
        }

        fn effect(self, room_name: &str) -> ServerPersistenceEffect {
            match self {
                Self::Save { version } => save_room_effect(
                    room_name,
                    &format!("{room_name}-version-{version}.mkv"),
                    version,
                ),
                Self::Delete { version } => ServerPersistenceEffect::DeleteRoom {
                    room_name: room_name.to_owned(),
                    version,
                },
            }
        }
    }

    #[derive(Debug)]
    struct RoomArbitrationModel {
        desired: ScheduledRoomMutation,
        ignored: Vec<ScheduledRoomMutation>,
    }

    impl RoomArbitrationModel {
        fn new(initial: ScheduledRoomMutation) -> Self {
            Self {
                desired: initial,
                ignored: Vec::new(),
            }
        }

        fn enqueue(&mut self, effect: ScheduledRoomMutation) {
            if effect.version() > self.desired.version() {
                self.ignored.push(self.desired);
                self.desired = effect;
            } else {
                self.ignored.push(effect);
            }
        }
    }

    fn seed_room_version_zero(store: &RoomPersistenceStore, room_name: &str) {
        let connection = store
            .connection("seed schedule baseline")
            .expect("schedule baseline connection should open");
        store
            .save_room(
                &connection,
                room_name,
                &PersistedRoomState {
                    files: vec![format!("{room_name}-baseline.mkv")],
                    index: Some(0),
                    position: 0.0,
                    last_activity_at_seconds: 0.0,
                    version: 0,
                    owner_bucket: None,
                    created_at_seconds: 0.0,
                },
            )
            .expect("schedule baseline should persist");
    }

    fn persisted_room_version_and_playlist(
        connection: &Connection,
        room_name: &str,
    ) -> Option<(i64, String)> {
        connection
            .query_row(
                "SELECT persistenceVersion, playlist FROM persistent_rooms WHERE name = ?1",
                [room_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("scheduled room row should be queryable")
    }

    const PERSISTENCE_CRASH_HELPER_TEST: &str =
        "persistence_actor::tests::persistence_crash_subprocess_helper";

    fn remove_sqlite_artifacts(db_path: &Path) {
        let path_text = db_path.to_string_lossy();
        for candidate in [
            db_path.to_path_buf(),
            PathBuf::from(format!("{path_text}-wal")),
            PathBuf::from(format!("{path_text}-shm")),
            PathBuf::from(format!("{path_text}-journal")),
        ] {
            match fs::remove_file(&candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!(
                    "temporary SQLite artifact '{}' should be removable: {error}",
                    candidate.display()
                ),
            }
        }
    }

    fn run_persistence_crash_helper(db_path: &Path, action: &str, point: &str) {
        let output = Command::new(
            std::env::current_exe().expect("current persistence test executable should resolve"),
        )
        .arg("--exact")
        .arg(PERSISTENCE_CRASH_HELPER_TEST)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(test_crash::HELPER_ENV, "1")
        .env(test_crash::ACTION_ENV, action)
        .env(test_crash::POINT_ENV, point)
        .env(test_crash::DB_PATH_ENV, db_path)
        .output()
        .expect("persistence crash helper should launch");
        assert_eq!(
            output.status.code(),
            Some(test_crash::EXIT_CODE),
            "persistence crash helper must terminate only at '{point}' for '{action}'\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_sqlite_integrity(db_path: &Path) {
        let connection =
            Connection::open(db_path).expect("crash-produced SQLite database should reopen");
        let result: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("SQLite integrity check should return a result");
        assert_eq!(
            result, "ok",
            "crash-produced SQLite database must remain structurally valid"
        );
    }

    fn sqlite_schema_columns(connection: &Connection) -> BTreeSet<String> {
        let mut statement = connection
            .prepare("PRAGMA table_info(persistent_rooms)")
            .expect("persistent room schema should be inspectable");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("persistent room schema should be queryable")
            .collect::<Result<BTreeSet<_>, _>>()
            .expect("persistent room schema rows should decode")
    }

    fn sqlite_table_exists(connection: &Connection, table_name: &str) -> bool {
        connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table_name],
                |_| Ok(()),
            )
            .optional()
            .expect("SQLite schema table lookup should succeed")
            .is_some()
    }

    fn baseline_room_state() -> PersistedRoomState {
        PersistedRoomState {
            files: vec!["baseline.mkv".to_owned()],
            index: Some(0),
            position: 11.5,
            last_activity_at_seconds: 12.5,
            version: 1,
            owner_bucket: Some("owner-v1".to_owned()),
            created_at_seconds: 13.5,
        }
    }

    #[derive(Debug, PartialEq)]
    struct RawPersistedRoomRow {
        playlist: String,
        playlist_json: String,
        playlist_index: Option<i64>,
        position: f64,
        last_activity_at_seconds: f64,
        persistence_version: i64,
        owner_bucket: Option<String>,
        created_at_seconds: f64,
    }

    fn raw_persisted_room_row(db_path: &Path, room_name: &str) -> RawPersistedRoomRow {
        Connection::open(db_path)
            .expect("persisted room database should open for raw inspection")
            .query_row(
                "SELECT playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                        persistenceVersion, ownerBucket, createdAt \
                 FROM persistent_rooms WHERE name = ?1",
                [room_name],
                |row| {
                    Ok(RawPersistedRoomRow {
                        playlist: row.get(0)?,
                        playlist_json: row.get(1)?,
                        playlist_index: row.get(2)?,
                        position: row.get(3)?,
                        last_activity_at_seconds: row.get(4)?,
                        persistence_version: row.get(5)?,
                        owner_bucket: row.get(6)?,
                        created_at_seconds: row.get(7)?,
                    })
                },
            )
            .expect("persisted room should remain queryable")
    }

    fn room_state_effect(room_name: &str, state: &PersistedRoomState) -> ServerPersistenceEffect {
        ServerPersistenceEffect::SaveRoom {
            room_name: room_name.to_owned(),
            files: state.files.clone(),
            playlist_index: state.index,
            position: state.position,
            last_activity_at_seconds: state.last_activity_at_seconds,
            owner_bucket: state.owner_bucket.clone(),
            created_at_seconds: state.created_at_seconds,
            version: state.version,
        }
    }

    fn sqlite_full_replacement_room_state(version: u64) -> PersistedRoomState {
        PersistedRoomState {
            files: (0..512)
                .map(|index| {
                    format!(
                        "replacement-{index:04}-{}.mkv",
                        "deterministic-payload-".repeat(256)
                    )
                })
                .collect(),
            index: Some(511),
            position: 987.75,
            last_activity_at_seconds: 2_002.5,
            version,
            owner_bucket: Some("owner-after-capacity-recovery".to_owned()),
            created_at_seconds: 902.5,
        }
    }

    fn replacement_room_state() -> PersistedRoomState {
        PersistedRoomState {
            files: vec!["new-a.mkv".to_owned(), "new-b.mkv".to_owned()],
            index: Some(1),
            position: 22.5,
            last_activity_at_seconds: 33.5,
            version: 2,
            owner_bucket: Some("owner-v2".to_owned()),
            created_at_seconds: 44.5,
        }
    }

    fn seed_baseline_room(db_path: &Path) {
        let store = RoomPersistenceStore::open(db_path)
            .expect("baseline room persistence schema should initialize");
        let connection = store
            .connection("seed crash-test room")
            .expect("baseline room connection should open");
        store
            .save_room(&connection, "room", &baseline_room_state())
            .expect("baseline room should persist");
    }

    fn seed_legacy_room_schema(db_path: &Path) {
        let connection =
            Connection::open(db_path).expect("legacy room database should be creatable");
        connection
            .execute_batch(
                "CREATE TABLE persistent_rooms (\
                    name STRING PRIMARY KEY, \
                    playlist STRING, \
                    playlistIndex INTEGER, \
                    position REAL, \
                    lastSavedUpdate REAL\
                )",
            )
            .expect("legacy room schema should be seedable");
        connection
            .execute(
                "INSERT INTO persistent_rooms \
                 (name, playlist, playlistIndex, position, lastSavedUpdate) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["legacy-room", "one.mkv\ntwo.mkv", 0_i64, 7.5_f64, 8.5_f64],
            )
            .expect("legacy room row should be seedable");
    }

    fn seed_legacy_playlist_rows(db_path: &Path) {
        let store = RoomPersistenceStore::open(db_path)
            .expect("room persistence schema should initialize before migration seed");
        let connection = store
            .connection("seed legacy playlist rows")
            .expect("legacy playlist seed connection should open");
        for (name, playlist, playlist_index) in [
            ("alpha-room", "alpha.mkv\nbeta.mkv", 9_i64),
            ("beta-room", "gamma.mkv", -4_i64),
        ] {
            connection
                .execute(
                    "INSERT INTO persistent_rooms \
                     (name, playlist, playlistJson, playlistIndex, position, lastSavedUpdate, \
                      persistenceVersion, ownerBucket, createdAt) \
                     VALUES (?1, ?2, NULL, ?3, 0, 0, 0, NULL, 0)",
                    params![name, playlist, playlist_index],
                )
                .expect("legacy playlist row should be seedable");
        }
    }

    fn persisted_playlist_migration_rows(
        db_path: &Path,
    ) -> Vec<(String, Option<String>, Option<i64>)> {
        let connection =
            Connection::open(db_path).expect("persisted playlist rows should be inspectable");
        let mut statement = connection
            .prepare(
                "SELECT name, playlistJson, playlistIndex \
                 FROM persistent_rooms ORDER BY name",
            )
            .expect("persisted playlist migration query should prepare");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("persisted playlist migration rows should be queryable")
            .collect::<Result<Vec<_>, _>>()
            .expect("persisted playlist migration rows should decode")
    }

    fn persisted_stats_versions(db_path: &Path, snapshot_time: i64) -> Vec<String> {
        let connection =
            Connection::open(db_path).expect("persisted stats rows should be inspectable");
        let mut statement = connection
            .prepare(
                "SELECT version FROM clients_snapshots \
                 WHERE snapshot_time = ?1 ORDER BY rowid",
            )
            .expect("persisted stats query should prepare");
        statement
            .query_map([snapshot_time], |row| row.get(0))
            .expect("persisted stats rows should be queryable")
            .collect::<Result<Vec<_>, _>>()
            .expect("persisted stats rows should decode")
    }

    fn persisted_quota_secret(db_path: &Path) -> Option<Vec<u8>> {
        Connection::open(db_path)
            .expect("quota metadata database should reopen")
            .query_row(
                "SELECT value FROM persistence_metadata WHERE key = 'quota-secret-v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .expect("quota-secret metadata should be queryable")
    }

    #[test]
    fn persistence_crash_subprocess_helper() {
        if std::env::var_os(test_crash::HELPER_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
            return;
        }

        let action = std::env::var(test_crash::ACTION_ENV)
            .expect("persistence crash helper action should be supplied");
        let point = std::env::var(test_crash::POINT_ENV)
            .expect("persistence crash helper point should be supplied");
        let db_path = PathBuf::from(
            std::env::var_os(test_crash::DB_PATH_ENV)
                .expect("persistence crash helper database path should be supplied"),
        );

        match action.as_str() {
            "schema" => {
                RoomPersistenceStore::open(&db_path)
                    .expect("schema crash helper should reach its armed point");
            }
            "room-migration" => {
                RoomPersistenceStore::open(&db_path)
                    .expect("room migration store should open")
                    .load_rooms()
                    .expect("room migration crash helper should reach its armed point");
            }
            "room-save" | "room-delete" => {
                let store = RoomPersistenceStore::open(&db_path)
                    .expect("room effect crash helper store should open");
                let (events, _) = broadcast::channel(8);
                let service =
                    RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
                        .expect("room effect crash helper worker should start");
                if action == "room-save" {
                    let state = replacement_room_state();
                    service.enqueue(ServerPersistenceEffect::SaveRoom {
                        room_name: "room".to_owned(),
                        files: state.files,
                        playlist_index: state.index,
                        position: state.position,
                        last_activity_at_seconds: state.last_activity_at_seconds,
                        owner_bucket: state.owner_bucket,
                        created_at_seconds: state.created_at_seconds,
                        version: state.version,
                    });
                } else {
                    service.enqueue(ServerPersistenceEffect::DeleteRoom {
                        room_name: "room".to_owned(),
                        version: 2,
                    });
                }
                let _ = service.flush();
            }
            "stats" => {
                let store = StatsPersistenceStore::open(&db_path)
                    .expect("stats crash helper store should open");
                let (events, _) = broadcast::channel(8);
                let service =
                    StatsPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
                        .expect("stats crash helper worker should start");
                service.enqueue(ServerPersistenceEffect::RecordStatsSnapshot {
                    snapshot_time: 41,
                    versions: vec!["1.0.0".to_owned(), "2.0.0".to_owned(), "3.0.0".to_owned()],
                });
                let _ = service.flush();
            }
            "quota-secret" => {
                RoomPersistenceStore::open(&db_path)
                    .expect("quota-secret crash helper store should open")
                    .load_or_create_quota_secret()
                    .expect("quota-secret crash helper should reach its armed point");
            }
            _ => panic!("unknown persistence crash helper action '{action}'"),
        }

        panic!("persistence crash helper did not terminate at armed point '{point}'");
    }

    #[test]
    fn room_schema_migration_recovers_after_every_process_interruption() {
        let stages = [
            test_crash::SCHEMA_AFTER_PLAYLIST_JSON,
            test_crash::SCHEMA_AFTER_PERSISTENCE_VERSION,
            test_crash::SCHEMA_AFTER_OWNER_BUCKET,
            test_crash::SCHEMA_AFTER_CREATED_AT,
            test_crash::SCHEMA_AFTER_METADATA,
        ];
        let added_room_columns = [
            "playlistJson",
            "persistenceVersion",
            "ownerBucket",
            "createdAt",
        ];

        for (stage_index, point) in stages.into_iter().enumerate() {
            let db_path = temporary_sqlite_path(&format!("schema-crash-{stage_index}"));
            remove_sqlite_artifacts(&db_path);
            seed_legacy_room_schema(&db_path);

            run_persistence_crash_helper(&db_path, "schema", point);
            assert_sqlite_integrity(&db_path);

            {
                let connection = Connection::open(&db_path)
                    .expect("partially migrated room schema should reopen");
                let columns = sqlite_schema_columns(&connection);
                for (column_index, column) in added_room_columns.iter().enumerate() {
                    assert_eq!(
                        columns.contains(*column),
                        column_index <= stage_index,
                        "interrupted schema at '{point}' must contain exactly the committed migration prefix"
                    );
                }
                assert_eq!(
                    sqlite_table_exists(&connection, "persistence_metadata"),
                    stage_index == stages.len() - 1,
                    "metadata schema visibility must match the exact interruption point"
                );
            }

            let first_reopen = RoomPersistenceStore::open(&db_path)
                .expect("production schema initialization should resume after interruption");
            let first_rooms = first_reopen
                .load_rooms()
                .expect("legacy row should load after schema recovery");
            assert_eq!(
                first_rooms.get("legacy-room"),
                Some(&PersistedRoomState {
                    files: vec!["one.mkv".to_owned(), "two.mkv".to_owned()],
                    index: Some(0),
                    position: 7.5,
                    last_activity_at_seconds: 8.5,
                    version: 0,
                    owner_bucket: None,
                    created_at_seconds: 0.0,
                }),
                "schema recovery must preserve the complete legacy row at '{point}'"
            );

            let second_reopen = RoomPersistenceStore::open(&db_path)
                .expect("schema recovery should be idempotent on a second reopen");
            assert_eq!(
                second_reopen
                    .load_rooms()
                    .expect("recovered row should remain readable"),
                first_rooms,
                "schema recovery must be idempotent at '{point}'"
            );
            let connection = second_reopen
                .connection("inspect recovered schema")
                .expect("recovered schema connection should open");
            let columns = sqlite_schema_columns(&connection);
            assert!(
                added_room_columns
                    .iter()
                    .all(|column| columns.contains(*column)),
                "recovery must finish every required room column at '{point}'"
            );
            assert!(
                sqlite_table_exists(&connection, "persistence_metadata"),
                "recovery must finish the metadata schema at '{point}'"
            );
            drop(connection);
            drop(second_reopen);
            drop(first_reopen);
            remove_sqlite_artifacts(&db_path);
        }
    }

    #[test]
    fn playlist_migration_is_atomic_across_process_interruption() {
        for (point, committed) in [
            (test_crash::ROOM_MIGRATION_AFTER_ROW, false),
            (test_crash::ROOM_MIGRATION_AFTER_COMMIT, true),
        ] {
            let db_path = temporary_sqlite_path(&format!("playlist-migration-crash-{committed}"));
            remove_sqlite_artifacts(&db_path);
            seed_legacy_playlist_rows(&db_path);

            run_persistence_crash_helper(&db_path, "room-migration", point);
            assert_sqlite_integrity(&db_path);

            let rows_after_crash = persisted_playlist_migration_rows(&db_path);
            let expected_after_crash = if committed {
                vec![
                    (
                        "alpha-room".to_owned(),
                        Some(r#"["alpha.mkv","beta.mkv"]"#.to_owned()),
                        Some(1),
                    ),
                    (
                        "beta-room".to_owned(),
                        Some(r#"["gamma.mkv"]"#.to_owned()),
                        Some(0),
                    ),
                ]
            } else {
                vec![
                    ("alpha-room".to_owned(), None, Some(9)),
                    ("beta-room".to_owned(), None, Some(-4)),
                ]
            };
            assert_eq!(
                rows_after_crash, expected_after_crash,
                "playlist migration must reopen as entirely old or entirely committed at '{point}'"
            );

            let store = RoomPersistenceStore::open(&db_path)
                .expect("playlist migration store should reopen after interruption");
            let recovered = store
                .load_rooms()
                .expect("playlist migration should recover after interruption");
            assert_eq!(recovered["alpha-room"].files, vec!["alpha.mkv", "beta.mkv"]);
            assert_eq!(recovered["alpha-room"].index, Some(1));
            assert_eq!(recovered["beta-room"].files, vec!["gamma.mkv"]);
            assert_eq!(recovered["beta-room"].index, Some(0));
            assert_eq!(
                store
                    .load_rooms()
                    .expect("playlist migration recovery should be idempotent"),
                recovered
            );
            assert_eq!(
                persisted_playlist_migration_rows(&db_path),
                vec![
                    (
                        "alpha-room".to_owned(),
                        Some(r#"["alpha.mkv","beta.mkv"]"#.to_owned()),
                        Some(1),
                    ),
                    (
                        "beta-room".to_owned(),
                        Some(r#"["gamma.mkv"]"#.to_owned()),
                        Some(0),
                    ),
                ],
                "recovery must finish a complete canonical playlist migration"
            );

            drop(store);
            remove_sqlite_artifacts(&db_path);
        }
    }

    #[test]
    fn room_effects_reopen_as_old_or_new_complete_state_across_commit_crashes() {
        for action in ["room-save", "room-delete"] {
            for (point, committed) in [
                (test_crash::ROOM_EFFECT_AFTER_WRITE, false),
                (test_crash::ROOM_EFFECT_AFTER_COMMIT, true),
            ] {
                let db_path = temporary_sqlite_path(&format!("{action}-crash-{committed}"));
                remove_sqlite_artifacts(&db_path);
                seed_baseline_room(&db_path);

                run_persistence_crash_helper(&db_path, action, point);
                assert_sqlite_integrity(&db_path);

                let first_reopen = RoomPersistenceStore::open(&db_path)
                    .expect("room effect database should reopen after interruption");
                let rooms = first_reopen
                    .load_rooms()
                    .expect("room effect database should load after interruption");
                let expected = match (action, committed) {
                    ("room-save", true) => Some(replacement_room_state()),
                    ("room-delete", true) => None,
                    _ => Some(baseline_room_state()),
                };
                assert_eq!(
                    rooms.get("room").cloned(),
                    expected,
                    "room effect must reopen as the complete old or new state for '{action}' at '{point}'"
                );

                let second_reopen = RoomPersistenceStore::open(&db_path)
                    .expect("room effect database should reopen twice");
                assert_eq!(
                    second_reopen
                        .load_rooms()
                        .expect("room effect state should remain readable"),
                    rooms,
                    "room effect recovery must be idempotent for '{action}' at '{point}'"
                );

                drop(second_reopen);
                drop(first_reopen);
                remove_sqlite_artifacts(&db_path);
            }
        }
    }

    #[test]
    fn stats_snapshots_reopen_as_empty_or_complete_across_commit_crashes() {
        for (point, expected_versions) in [
            (test_crash::STATS_AFTER_FIRST_ROW, Vec::<String>::new()),
            (
                test_crash::STATS_AFTER_COMMIT,
                vec!["1.0.0".to_owned(), "2.0.0".to_owned(), "3.0.0".to_owned()],
            ),
        ] {
            let db_path = temporary_sqlite_path(&format!("stats-crash-{point}"));
            remove_sqlite_artifacts(&db_path);
            StatsPersistenceStore::open(&db_path)
                .expect("stats persistence schema should initialize");

            run_persistence_crash_helper(&db_path, "stats", point);
            assert_sqlite_integrity(&db_path);
            assert_eq!(
                persisted_stats_versions(&db_path, 41),
                expected_versions,
                "stats snapshot must reopen with zero or every row at '{point}'"
            );

            let store = StatsPersistenceStore::open(&db_path)
                .expect("stats store should reopen after interruption");
            let mut connection = store
                .connection("write recovery stats snapshot")
                .expect("stats recovery connection should open");
            store
                .add_version_logs(
                    &mut connection,
                    42,
                    &["4.0.0".to_owned(), "5.0.0".to_owned()],
                )
                .expect("stats writes should recover after interruption");
            drop(connection);
            drop(store);
            assert_eq!(
                persisted_stats_versions(&db_path, 42),
                vec!["4.0.0", "5.0.0"],
                "stats recovery must accept a complete later snapshot at '{point}'"
            );

            remove_sqlite_artifacts(&db_path);
        }
    }

    #[test]
    fn quota_secret_creation_reopens_without_partial_metadata_across_crashes() {
        for (point, insert_committed) in [
            (test_crash::QUOTA_SECRET_AFTER_GENERATE, false),
            (test_crash::QUOTA_SECRET_AFTER_INSERT, true),
        ] {
            let db_path = temporary_sqlite_path(&format!("quota-secret-crash-{insert_committed}"));
            remove_sqlite_artifacts(&db_path);
            RoomPersistenceStore::open(&db_path)
                .expect("quota-secret persistence schema should initialize");

            run_persistence_crash_helper(&db_path, "quota-secret", point);
            assert_sqlite_integrity(&db_path);
            let secret_after_crash = persisted_quota_secret(&db_path);
            assert_eq!(
                secret_after_crash.is_some(),
                insert_committed,
                "quota-secret row visibility must match its commit boundary at '{point}'"
            );
            if let Some(secret) = secret_after_crash.as_ref() {
                assert_eq!(
                    secret.len(),
                    32,
                    "a committed quota secret must never be partial"
                );
            }

            let first_reopen = RoomPersistenceStore::open(&db_path)
                .expect("quota-secret store should reopen after interruption");
            let recovered_secret = first_reopen
                .load_or_create_quota_secret()
                .expect("quota-secret creation should recover after interruption");
            if let Some(committed_secret) = secret_after_crash {
                assert_eq!(
                    recovered_secret.as_slice(),
                    committed_secret,
                    "a committed quota secret must remain authoritative"
                );
            }
            let second_reopen = RoomPersistenceStore::open(&db_path)
                .expect("quota-secret store should reopen twice");
            assert_eq!(
                second_reopen
                    .load_or_create_quota_secret()
                    .expect("recovered quota secret should reload"),
                recovered_secret,
                "quota-secret recovery must converge idempotently"
            );
            assert_eq!(
                persisted_quota_secret(&db_path)
                    .expect("recovered quota secret should be durable")
                    .len(),
                32,
                "recovered quota metadata must contain exactly one complete secret"
            );

            drop(second_reopen);
            drop(first_reopen);
            remove_sqlite_artifacts(&db_path);
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
    fn room_worker_start_classifies_filesystem_path_open_failure() {
        let db_path = temporary_sqlite_path("room-worker-path-open-failure");
        remove_sqlite_artifacts(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize before path replacement");
        remove_sqlite_artifacts(&db_path);
        fs::create_dir(&db_path)
            .expect("a directory should replace the database file for deterministic open failure");

        let (events, _) = broadcast::channel(8);
        let error = RoomPersistenceService::start(store, events, Arc::new(AtomicUsize::new(0)))
            .expect_err("the worker-owned connection must reject a directory as its file");
        let RoomPersistenceError::Sqlite {
            action,
            path,
            source,
        } = error;
        assert_eq!(action, "connect persistence worker");
        assert_eq!(path, db_path);
        let rusqlite::Error::SqliteFailure(sqlite_error, message) = source else {
            panic!("worker connection should expose a classified SQLite open failure");
        };
        assert_eq!(
            sqlite_error.code,
            ErrorCode::CannotOpen,
            "the real filesystem boundary should classify the path collision as SQLITE_CANTOPEN"
        );
        assert!(
            message
                .as_deref()
                .is_some_and(|message| message.contains("unable to open database file")),
            "the SQLite VFS should retain its concrete open-failure context"
        );

        fs::remove_dir(&db_path).expect("temporary database-path directory should be removable");
    }

    #[test]
    fn room_worker_sqlite_full_preserves_old_row_and_recovers_on_same_connection() {
        let db_path = temporary_sqlite_path("room-worker-sqlite-full-recovery");
        remove_sqlite_artifacts(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let baseline = PersistedRoomState {
            version: 41,
            ..baseline_room_state()
        };
        let seed_connection = store
            .connection("seed worker SQLITE_FULL baseline")
            .expect("baseline connection should open");
        store
            .save_room(&seed_connection, "room", &baseline)
            .expect("baseline room should persist");
        let checkpoint_busy: i64 = seed_connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .expect("baseline WAL should checkpoint");
        assert_eq!(checkpoint_busy, 0, "baseline checkpoint must not be busy");
        drop(seed_connection);
        let durable_before_failure = raw_persisted_room_row(&db_path, "room");

        let connection_limits = Arc::new(Mutex::new(Vec::new()));
        let hook_connection_limits = Arc::clone(&connection_limits);
        let hooks = RoomPersistenceWorkerHooks {
            before_transaction: Some(Arc::new(move |connection, effect| {
                let (_, version) = room_effect_key_and_version(effect)
                    .expect("the room worker hook should receive only room effects");
                let page_count: i64 = connection
                    .query_row("PRAGMA page_count", [], |row| row.get(0))
                    .expect("worker-owned connection page count should be queryable");
                let requested_limit = if version == 42 {
                    page_count
                } else {
                    page_count + 4_096
                };
                connection
                    .pragma_update(None, "max_page_count", requested_limit)
                    .expect("worker-owned connection page limit should update");
                let effective_limit: i64 = connection
                    .query_row("PRAGMA max_page_count", [], |row| row.get(0))
                    .expect("worker-owned connection page limit should be queryable");
                hook_connection_limits
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((version, page_count, effective_limit));
            })),
            ..RoomPersistenceWorkerHooks::default()
        };
        let (events, _) = broadcast::channel(64);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
            store,
            events,
            Arc::clone(&degraded_worker_count),
            4,
            hooks,
        )
        .expect("controlled room persistence worker should start");

        let failed_replacement = sqlite_full_replacement_room_state(42);
        service.enqueue(room_state_effect("room", &failed_replacement));
        assert!(
            !service.flush(),
            "SQLITE_FULL must remain an unresolved worker failure"
        );
        assert_eq!(
            degraded_worker_count.load(Ordering::Acquire),
            1,
            "the room worker must enter degraded mode exactly once"
        );
        let failure_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(failure_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Failed {
                effect: ServerPersistenceEffect::SaveRoom { version: 42, .. },
                error,
                ..
            } if error.contains("database or disk is full")
        )));
        assert_eq!(
            failure_events
                .iter()
                .filter(|event| matches!(event, ServerPersistenceEvent::Degraded { .. }))
                .count(),
            1,
            "retries of one unresolved capacity failure must not duplicate degradation"
        );
        assert!(!failure_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { version: 42, .. },
                ..
            }
        )));
        assert_eq!(
            raw_persisted_room_row(&db_path, "room"),
            durable_before_failure,
            "the worker transaction must roll back every persisted column after SQLITE_FULL"
        );
        assert_sqlite_integrity(&db_path);
        let constrained_observations = connection_limits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(
            constrained_observations
                .iter()
                .any(|(version, page_count, limit)| version == &42 && limit == page_count),
            "the failure must be imposed on the actor-owned connection with no page headroom"
        );

        let recovered_replacement = sqlite_full_replacement_room_state(43);
        service.enqueue(room_state_effect("room", &recovered_replacement));
        assert!(
            service.flush(),
            "a newer desired state should persist after capacity is restored on the same worker connection"
        );
        assert_eq!(
            degraded_worker_count.load(Ordering::Acquire),
            0,
            "successful forward progress should clear worker degradation"
        );
        let recovery_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(recovery_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { version: 43, .. },
                ..
            }
        )));
        assert_eq!(
            recovery_events
                .iter()
                .filter(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
                .count(),
            1,
            "the successful replacement should report one recovery transition"
        );
        assert!(
            connection_limits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|(version, page_count, limit)| version == &43 && limit > page_count),
            "recovery must lift the limit on the same actor-owned connection"
        );
        drop(service);

        let reopened = RoomPersistenceStore::open(&db_path)
            .expect("room persistence should reopen after worker recovery");
        assert_eq!(
            reopened
                .load_rooms()
                .expect("recovered room state should load")
                .get("room"),
            Some(&recovered_replacement),
            "normal reopen must expose the complete newer room state"
        );
        assert_sqlite_integrity(&db_path);
        drop(reopened);
        remove_sqlite_artifacts(&db_path);
    }

    #[test]
    fn room_worker_read_only_connection_preserves_old_row_and_recovers() {
        let db_path = temporary_sqlite_path("room-worker-read-only-recovery");
        remove_sqlite_artifacts(&db_path);
        let store = RoomPersistenceStore::open(&db_path)
            .expect("room persistence schema should initialize");
        let baseline = baseline_room_state();
        let seed_connection = store
            .connection("seed worker read-only baseline")
            .expect("baseline connection should open");
        store
            .save_room(&seed_connection, "room", &baseline)
            .expect("baseline room should persist");
        drop(seed_connection);
        let durable_before_failure = raw_persisted_room_row(&db_path, "room");

        let query_only_observations = Arc::new(Mutex::new(Vec::new()));
        let hook_query_only_observations = Arc::clone(&query_only_observations);
        let hooks = RoomPersistenceWorkerHooks {
            before_transaction: Some(Arc::new(move |connection, effect| {
                let (_, version) = room_effect_key_and_version(effect)
                    .expect("the room worker hook should receive only room effects");
                connection
                    .pragma_update(None, "query_only", version == 2)
                    .expect("worker-owned connection query-only mode should update");
                let query_only: bool = connection
                    .query_row("PRAGMA query_only", [], |row| row.get(0))
                    .expect("worker-owned connection query-only mode should be queryable");
                hook_query_only_observations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((version, query_only));
            })),
            ..RoomPersistenceWorkerHooks::default()
        };
        let (events, _) = broadcast::channel(64);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
            store,
            events,
            Arc::clone(&degraded_worker_count),
            4,
            hooks,
        )
        .expect("controlled room persistence worker should start");

        let denied = replacement_room_state();
        service.enqueue(room_state_effect("room", &denied));
        assert!(
            !service.flush(),
            "a write-denied worker connection must retain an unresolved desired state"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        let failure_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(failure_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Failed {
                effect: ServerPersistenceEffect::SaveRoom { version: 2, .. },
                error,
                ..
            } if error.contains("readonly database")
        )));
        assert_eq!(
            raw_persisted_room_row(&db_path, "room"),
            durable_before_failure,
            "a write-denied worker transaction must preserve all baseline columns"
        );
        assert_sqlite_integrity(&db_path);
        assert!(
            query_only_observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|observation| observation == &(2, true)),
            "the failure must occur with the actor-owned connection in query-only mode"
        );

        let recovered = PersistedRoomState {
            files: vec!["recovered.mkv".to_owned()],
            index: Some(0),
            position: 55.5,
            last_activity_at_seconds: 66.5,
            version: 3,
            owner_bucket: Some("owner-after-write-access".to_owned()),
            created_at_seconds: 77.5,
        };
        service.enqueue(room_state_effect("room", &recovered));
        assert!(
            service.flush(),
            "a newer desired state should persist once write access returns"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);
        let recovery_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(recovery_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { version: 3, .. },
                ..
            }
        )));
        assert_eq!(
            recovery_events
                .iter()
                .filter(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
                .count(),
            1
        );
        assert!(
            query_only_observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|observation| observation == &(3, false)),
            "recovery must restore write access on the same actor-owned connection"
        );
        drop(service);

        let reopened = RoomPersistenceStore::open(&db_path)
            .expect("room persistence should reopen after write-access recovery");
        assert_eq!(
            reopened
                .load_rooms()
                .expect("recovered room state should load")
                .get("room"),
            Some(&recovered)
        );
        assert_sqlite_integrity(&db_path);
        drop(reopened);
        remove_sqlite_artifacts(&db_path);
    }

    #[test]
    fn room_worker_pre_transaction_arbitration_matches_reference_model_for_all_effect_pairs() {
        let schedules = [
            (
                ScheduledRoomMutation::Save { version: 1 },
                ScheduledRoomMutation::Save { version: 2 },
            ),
            (
                ScheduledRoomMutation::Save { version: 1 },
                ScheduledRoomMutation::Delete { version: 2 },
            ),
            (
                ScheduledRoomMutation::Delete { version: 2 },
                ScheduledRoomMutation::Save { version: 3 },
            ),
            (
                ScheduledRoomMutation::Save { version: 2 },
                ScheduledRoomMutation::Save { version: 1 },
            ),
            (
                ScheduledRoomMutation::Delete { version: 2 },
                ScheduledRoomMutation::Save { version: 1 },
            ),
            (
                ScheduledRoomMutation::Save { version: 2 },
                ScheduledRoomMutation::Delete { version: 2 },
            ),
        ];

        for (schedule_index, (initial, concurrent)) in schedules.into_iter().enumerate() {
            let db_path = temporary_sqlite_path(&format!("room-pre-transaction-{schedule_index}"));
            remove_sqlite_artifacts(&db_path);
            let store = RoomPersistenceStore::open(&db_path)
                .expect("room schedule schema should initialize");
            seed_room_version_zero(&store, "room");
            let (events, _) = broadcast::channel(64);
            let mut event_rx = events.subscribe();
            let (hooks, controller, transaction_entries) = controlled_room_worker_hooks();
            let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
                store,
                events,
                Arc::new(AtomicUsize::new(0)),
                1,
                hooks,
            )
            .expect("controlled room persistence worker should start");
            let checkpoint = RoomWorkerTestCheckpoint::BeforeArbitration {
                room_name: "room".to_owned(),
                version: initial.version(),
            };
            controller.arm(checkpoint.clone());

            service.enqueue(initial.effect("room"));
            controller.wait_for(&checkpoint);
            service.enqueue(concurrent.effect("room"));
            controller.release();
            assert!(
                service.flush(),
                "schedule {schedule_index} should settle its newest desired state"
            );

            let mut model = RoomArbitrationModel::new(initial);
            model.enqueue(concurrent);
            assert_eq!(
                *transaction_entries
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                vec![model.desired.effect("room")],
                "schedule {schedule_index}: only the reference-model winner may enter a transaction"
            );
            let connection =
                Connection::open(&db_path).expect("scheduled room database should reopen");
            let durable = persisted_room_version_and_playlist(&connection, "room");
            match model.desired {
                ScheduledRoomMutation::Save { version } => assert_eq!(
                    durable,
                    Some((
                        i64::try_from(version).expect("test version should fit SQLite i64"),
                        format!("room-version-{version}.mkv"),
                    )),
                    "schedule {schedule_index}: newest save should be durable"
                ),
                ScheduledRoomMutation::Delete { .. } => assert_eq!(
                    durable, None,
                    "schedule {schedule_index}: newest delete should be durable"
                ),
            }
            let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
            for ignored in model.ignored {
                assert!(
                    events.iter().any(|event| matches!(
                        event,
                        ServerPersistenceEvent::IgnoredStale { effect, .. }
                            if effect == &ignored.effect("room")
                    )),
                    "schedule {schedule_index}: the losing {ignored:?} effect should be reported stale"
                );
            }

            drop(service);
            drop(connection);
            remove_sqlite_artifacts(&db_path);
        }
    }

    #[test]
    fn room_worker_bounded_wake_queue_coalesces_cross_room_updates_before_transaction_entry() {
        let db_path = temporary_sqlite_path("room-global-pre-transaction");
        remove_sqlite_artifacts(&db_path);
        let store =
            RoomPersistenceStore::open(&db_path).expect("room schedule schema should initialize");
        seed_room_version_zero(&store, "room-a");
        seed_room_version_zero(&store, "room-b");
        let (events, _) = broadcast::channel(64);
        let mut event_rx = events.subscribe();
        let (hooks, controller, transaction_entries) = controlled_room_worker_hooks();
        let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
            store,
            events,
            Arc::new(AtomicUsize::new(0)),
            1,
            hooks,
        )
        .expect("bounded room persistence worker should start");

        controller.arm(RoomWorkerTestCheckpoint::BeforeScan);
        service.enqueue(save_room_effect("room-a", "room-a-v1.mkv", 1));
        controller.wait_for(&RoomWorkerTestCheckpoint::BeforeScan);
        service.enqueue(save_room_effect("room-b", "room-b-v1.mkv", 1));
        let stale_scan_checkpoint = RoomWorkerTestCheckpoint::BeforeArbitration {
            room_name: "room-a".to_owned(),
            version: 1,
        };
        controller.arm(stale_scan_checkpoint.clone());
        controller.release();

        controller.wait_for(&stale_scan_checkpoint);
        service.enqueue(save_room_effect("room-a", "room-a-v2.mkv", 2));
        service.enqueue(ServerPersistenceEffect::DeleteRoom {
            room_name: "room-b".to_owned(),
            version: 2,
        });
        controller.release();
        assert!(
            service.flush(),
            "coalesced cross-room desired state should flush"
        );

        assert_eq!(
            *transaction_entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![
                save_room_effect("room-a", "room-a-v2.mkv", 2),
                ServerPersistenceEffect::DeleteRoom {
                    room_name: "room-b".to_owned(),
                    version: 2,
                },
            ],
            "the full wake queue must not force either stale snapshot into a transaction"
        );
        let connection =
            Connection::open(&db_path).expect("cross-room schedule database should reopen");
        assert_eq!(
            persisted_room_version_and_playlist(&connection, "room-a"),
            Some((2, "room-a-v2.mkv".to_owned()))
        );
        assert_eq!(
            persisted_room_version_and_playlist(&connection, "room-b"),
            None
        );
        let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        for (room_name, version) in [("room-a", 1), ("room-b", 1)] {
            assert!(events.iter().any(|event| matches!(
                event,
                ServerPersistenceEvent::IgnoredStale { effect, .. }
                    if super::room_effect_key_and_version(effect)
                        == Some((room_name, version))
            )));
        }
        assert!(
            !events.iter().any(|event| matches!(
                event,
                ServerPersistenceEvent::Failed { .. } | ServerPersistenceEvent::Degraded { .. }
            )),
            "a full coalescible Wake queue is healthy while desired state remains retained"
        );

        drop(service);
        drop(connection);
        remove_sqlite_artifacts(&db_path);
    }

    #[test]
    fn room_worker_post_arbitration_generation_change_rolls_back_before_stale_commit() {
        let db_path = temporary_sqlite_path("room-post-arbitration-rollback");
        remove_sqlite_artifacts(&db_path);
        let store =
            RoomPersistenceStore::open(&db_path).expect("room schedule schema should initialize");
        seed_room_version_zero(&store, "room");
        let (events, _) = broadcast::channel(32);
        let mut event_rx = events.subscribe();
        let (hooks, controller, transaction_entries) = controlled_room_worker_hooks();
        let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
            store,
            events,
            Arc::new(AtomicUsize::new(0)),
            1,
            hooks,
        )
        .expect("controlled room persistence worker should start");
        let checkpoint = RoomWorkerTestCheckpoint::AfterArbitration {
            room_name: "room".to_owned(),
            version: 1,
        };
        controller.arm(checkpoint.clone());

        service.enqueue(save_room_effect("room", "stale-v1.mkv", 1));
        controller.wait_for(&checkpoint);
        service.enqueue(save_room_effect("room", "current-v2.mkv", 2));
        controller.release();
        assert!(
            service.flush(),
            "new desired state should commit after stale transaction rollback"
        );

        assert_eq!(
            *transaction_entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![
                save_room_effect("room", "stale-v1.mkv", 1),
                save_room_effect("room", "current-v2.mkv", 2),
            ],
            "the post-arbitration race enters the old transaction but must retry the replacement"
        );
        let connection =
            Connection::open(&db_path).expect("post-arbitration database should reopen");
        assert_eq!(
            persisted_room_version_and_playlist(&connection, "room"),
            Some((2, "current-v2.mkv".to_owned())),
            "the stale write must roll back before commit"
        );
        let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::IgnoredStale {
                effect: ServerPersistenceEffect::SaveRoom { version: 1, .. },
                ..
            }
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { version: 1, .. },
                ..
            }
        )));

        drop(service);
        drop(connection);
        remove_sqlite_artifacts(&db_path);
    }

    #[test]
    fn room_worker_concurrent_replacement_of_failed_effect_recovers_without_retrying_stale_work() {
        let db_path = temporary_sqlite_path("room-failure-concurrent-replacement");
        remove_sqlite_artifacts(&db_path);
        let store =
            RoomPersistenceStore::open(&db_path).expect("room schedule schema should initialize");
        let (events, _) = broadcast::channel(64);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let (hooks, controller, transaction_entries) = controlled_room_worker_hooks();
        let service = RoomPersistenceService::start_with_queue_capacity_and_hooks(
            store,
            events,
            Arc::clone(&degraded_worker_count),
            1,
            hooks,
        )
        .expect("controlled room persistence worker should start");
        let external = Connection::open(&db_path).expect("failure injector should connect");
        external
            .execute_batch(
                "CREATE TRIGGER fail_room_v1 \
                 BEFORE INSERT ON persistent_rooms \
                 WHEN NEW.name = 'room' AND NEW.persistenceVersion = 1 \
                 BEGIN \
                     SELECT RAISE(FAIL, 'injected version-1 failure'); \
                 END",
            )
            .expect("version-1 failure trigger should install");

        service.enqueue(save_room_effect("room", "failed-v1.mkv", 1));
        assert!(!service.flush(), "the first desired effect should fail");
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 1);
        external
            .execute("DROP TRIGGER fail_room_v1", [])
            .expect("version-1 failure trigger should be removable");
        let checkpoint = RoomWorkerTestCheckpoint::BeforeArbitration {
            room_name: "room".to_owned(),
            version: 1,
        };
        controller.arm(checkpoint.clone());
        assert!(service.worker.wake(), "failed desired state should wake");
        controller.wait_for(&checkpoint);
        service.enqueue(save_room_effect("room", "replacement-v2.mkv", 2));
        controller.release();
        assert!(
            service.flush(),
            "successful concurrent replacement should resolve degradation"
        );

        assert_eq!(
            *transaction_entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![
                save_room_effect("room", "failed-v1.mkv", 1),
                save_room_effect("room", "failed-v1.mkv", 1),
                save_room_effect("room", "replacement-v2.mkv", 2),
            ],
            "enqueue and the first flush each attempt version 1, but the controlled stale retry must be rejected before transaction entry"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);
        let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        let recovered_count = events
            .iter()
            .filter(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
            .count();
        assert_eq!(
            recovered_count, 1,
            "the successful replacement should report exactly one recovery"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::IgnoredStale {
                effect: ServerPersistenceEffect::SaveRoom { version: 1, .. },
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied {
                effect: ServerPersistenceEffect::SaveRoom { version: 2, .. },
                ..
            }
        )));

        drop(service);
        drop(external);
        remove_sqlite_artifacts(&db_path);
    }

    #[test]
    fn stats_worker_bounded_queue_degrades_before_transaction_and_recovers_after_drain() {
        let db_path = temporary_sqlite_path("stats-bounded-pre-transaction");
        remove_sqlite_artifacts(&db_path);
        let store =
            StatsPersistenceStore::open(&db_path).expect("stats schedule schema should initialize");
        let (events, _) = broadcast::channel(32);
        let mut event_rx = events.subscribe();
        let degraded_worker_count = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let service = StatsPersistenceService::start_with_queue_capacity_and_start_hook(
            store,
            events,
            Arc::clone(&degraded_worker_count),
            1,
            move || {
                started_tx
                    .send(())
                    .expect("stats schedule controller should still be listening");
                release_rx
                    .recv_timeout(SCHEDULE_WATCHDOG)
                    .expect("stats schedule controller should release the worker");
            },
        )
        .expect("controlled stats persistence worker should start");
        started_rx
            .recv_timeout(SCHEDULE_WATCHDOG)
            .expect("stats worker should reach its pre-receive checkpoint");

        let retained = ServerPersistenceEffect::RecordStatsSnapshot {
            snapshot_time: 11,
            versions: vec!["retained".to_owned()],
        };
        let overflow = ServerPersistenceEffect::RecordStatsSnapshot {
            snapshot_time: 12,
            versions: vec!["overflow".to_owned()],
        };
        service.enqueue(retained.clone());
        service.enqueue(overflow.clone());
        assert_eq!(
            degraded_worker_count.load(Ordering::Acquire),
            1,
            "overflow must degrade before the blocked worker can enter a transaction"
        );
        let overflow_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(overflow_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Failed { effect, error, .. }
                if effect == &overflow && error == "persistence command queue is full"
        )));
        assert!(
            overflow_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Degraded { .. }))
        );

        release_tx
            .send(())
            .expect("blocked stats worker should still be waiting");
        assert!(
            service.flush(),
            "retained stats effect and flush should drain"
        );
        assert_eq!(degraded_worker_count.load(Ordering::Acquire), 0);
        assert_eq!(persisted_stats_versions(&db_path, 11), vec!["retained"]);
        assert!(
            persisted_stats_versions(&db_path, 12).is_empty(),
            "the rejected overflow effect must never enter a transaction"
        );
        let recovery_events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(recovery_events.iter().any(|event| matches!(
            event,
            ServerPersistenceEvent::Applied { effect, .. } if effect == &retained
        )));
        assert!(
            recovery_events
                .iter()
                .any(|event| matches!(event, ServerPersistenceEvent::Recovered { .. }))
        );

        drop(service);
        remove_sqlite_artifacts(&db_path);
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
