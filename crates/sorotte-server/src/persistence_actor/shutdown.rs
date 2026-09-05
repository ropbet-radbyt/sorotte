//! A worker owns its connection and transaction. The lifecycle owner supplies
//! one elapsed deadline shared by queue admission, busy retries and joining.
//! SQLite contention is interruptible; arbitrary blocking filesystem calls are
//! not. An exceptional unjoined worker retains its handle in an observable
//! registry rather than being detached or reported as durable.
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub(crate) struct WorkerControl {
    deadline: Mutex<Option<Instant>>,
    shutdown_deadline: Mutex<Option<Instant>>,
    pub(super) stop: AtomicBool,
    pub(super) wake_pending: AtomicBool,
}

impl WorkerControl {
    pub(crate) fn begin_shutdown(&self, deadline: Instant) {
        let mut current = self
            .shutdown_deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current = Some(current.map_or(deadline, |old| old.min(deadline)));
    }

    pub(super) fn effective_deadline(&self, deadline: Instant) -> Instant {
        self.shutdown_deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map_or(deadline, |shutdown| shutdown.min(deadline))
    }

    pub(super) fn set_deadline(&self, deadline: Option<Instant>) {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = deadline;
    }
    pub(super) fn expired(&self) -> bool {
        self.stop.load(Ordering::Acquire)
            || self
                .deadline
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some_and(|deadline| Instant::now() >= self.effective_deadline(deadline))
            || self
                .shutdown_deadline
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

thread_local! {
    static BUSY_CONTEXT: RefCell<Option<(Arc<WorkerControl>, Instant)>> = const { RefCell::new(None) };
}

pub(super) fn install_busy_handler(
    connection: &rusqlite::Connection,
    control: Arc<WorkerControl>,
) -> rusqlite::Result<()> {
    BUSY_CONTEXT.with(|context| *context.borrow_mut() = Some((control, Instant::now())));
    connection.busy_handler(Some(busy_handler))
}

fn busy_handler(attempt: i32) -> bool {
    BUSY_CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let Some((control, started)) = context.as_mut() else {
            return false;
        };
        if attempt == 0 {
            *started = Instant::now();
        }
        if control.expired() || started.elapsed() >= Duration::from_secs(5) {
            return false;
        }
        // SQLite calls this on the dedicated owner thread, never the actor.
        std::thread::sleep(Duration::from_millis(5));
        !control.expired()
    })
}

static UNJOINED: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
static ASYNC_CLEANUP: OnceLock<Mutex<Vec<tokio::task::JoinHandle<()>>>> = OnceLock::new();

pub(crate) fn retain_async_cleanup(handle: tokio::task::JoinHandle<()>) {
    ASYNC_CLEANUP
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(handle);
}

pub(super) fn retain_unjoined(handle: JoinHandle<()>) {
    UNJOINED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(handle);
}

/// Number of workers whose forced shutdown did not complete. Finished workers
/// are joined on observation. Nonzero is an explicit unresolved lifecycle fault.
pub fn persistence_workers_awaiting_join() -> usize {
    let mut handles = UNJOINED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let _ = handles.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
    let mut cleanup = ASYNC_CLEANUP
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cleanup.retain(|handle| !handle.is_finished());
    handles.len() + cleanup.len()
}
