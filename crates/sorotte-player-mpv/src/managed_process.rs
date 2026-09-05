//! Lifetime ownership for Sorotte-launched mpv processes. External attachments
//! never enter this scope. Shutdown can terminate a child independently of the
//! thread performing player I/O, and platform containment survives parent exit.

use std::{
    cell::RefCell,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::PlatformProcess;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::PlatformProcess;

const DROP_TIMEOUT: Duration = Duration::from_millis(500);

/// A deliberately small launch description. Environment overrides are applied
/// to the inherited environment; standard streams are always disconnected.
#[derive(Clone, Default)]
pub struct ManagedMpvCommand {
    program: PathBuf,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    environment: Vec<(OsString, OsString)>,
}

impl ManagedMpvCommand {
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_path_buf(),
            ..Self::default()
        }
    }

    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    pub fn current_dir(&mut self, directory: impl AsRef<Path>) -> &mut Self {
        self.current_dir = Some(directory.as_ref().to_path_buf());
        self
    }

    pub fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.environment
            .push((name.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    pub fn spawn(&self, ipc_cleanup_path: Option<PathBuf>) -> io::Result<OwnedMpvProcess> {
        let scope = CURRENT_SCOPE.with(|current| current.borrow().clone());
        if scope
            .as_ref()
            .is_some_and(ManagedMpvShutdownScope::is_stopping)
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "owned player launch cancelled by runtime shutdown",
            ));
        }
        let process = OwnedMpvProcess {
            entry: Arc::new(OwnedEntry {
                process: PlatformProcess::spawn(self)?,
                ipc_cleanup_path,
                ipc_cleaned: AtomicBool::new(false),
                cleanup: Mutex::new(()),
            }),
        };
        if let Some(scope) = scope {
            scope.register(&process)?;
        }
        Ok(process)
    }
}

#[derive(Debug)]
struct OwnedEntry {
    process: PlatformProcess,
    ipc_cleanup_path: Option<PathBuf>,
    ipc_cleaned: AtomicBool,
    cleanup: Mutex<()>,
}

impl OwnedEntry {
    fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        let _cleanup = loop {
            match self.cleanup.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "owned player cleanup is still running",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        if self.ipc_cleaned.load(Ordering::Acquire) {
            return Ok(());
        }
        self.process.terminate_until(deadline)?;
        if !self.ipc_cleaned.swap(true, Ordering::AcqRel)
            && let Some(path) = &self.ipc_cleanup_path
        {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct OwnedMpvProcess {
    entry: Arc<OwnedEntry>,
}

impl OwnedMpvProcess {
    pub fn id(&self) -> u32 {
        self.entry.process.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.entry.process.try_wait()
    }

    pub fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        self.entry.terminate_until(deadline)
    }

    /// Adopts a synthetic child for lifecycle fixtures. Production launches must
    /// use ManagedMpvCommand so containment is established before execution.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn from_test_child(
        child: std::process::Child,
        ipc_cleanup_path: Option<PathBuf>,
    ) -> io::Result<Self> {
        Ok(Self {
            entry: Arc::new(OwnedEntry {
                process: PlatformProcess::adopt(child)?,
                ipc_cleanup_path,
                ipc_cleaned: AtomicBool::new(false),
                cleanup: Mutex::new(()),
            }),
        })
    }
}

impl Drop for OwnedMpvProcess {
    fn drop(&mut self) {
        if let Err(error) = self.entry.terminate_until(Instant::now() + DROP_TIMEOUT) {
            eprintln!("owned mpv cleanup incomplete: {error}");
        }
    }
}

#[derive(Default)]
struct ScopeState {
    stopping: AtomicBool,
    entries: Mutex<Vec<Weak<OwnedEntry>>>,
}

/// Independent termination access scoped to one runtime, including replacement
/// processes launched while it runs. A stopped scope never admits another child.
#[derive(Clone, Default)]
pub struct ManagedMpvShutdownScope(Arc<ScopeState>);

thread_local! {
    static CURRENT_SCOPE: RefCell<Option<ManagedMpvShutdownScope>> = const { RefCell::new(None) };
}

pub struct ManagedMpvScopeEntry {
    previous: Option<ManagedMpvShutdownScope>,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for ManagedMpvScopeEntry {
    fn drop(&mut self) {
        CURRENT_SCOPE.with(|current| *current.borrow_mut() = self.previous.take());
    }
}

impl ManagedMpvShutdownScope {
    pub fn enter(&self) -> ManagedMpvScopeEntry {
        ManagedMpvScopeEntry {
            previous: CURRENT_SCOPE.with(|current| current.replace(Some(self.clone()))),
            _same_thread: std::marker::PhantomData,
        }
    }

    pub fn register(&self, process: &OwnedMpvProcess) -> io::Result<()> {
        {
            let mut entries = self
                .0
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.retain(|entry| entry.strong_count() != 0);
            entries.push(Arc::downgrade(&process.entry));
        }
        if self.is_stopping() {
            process.terminate_until(Instant::now() + DROP_TIMEOUT)?;
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "owned player registration cancelled by runtime shutdown",
            ));
        }
        Ok(())
    }

    pub fn request_shutdown(&self) {
        self.0.stopping.store(true, Ordering::Release);
    }

    pub fn is_stopping(&self) -> bool {
        self.0.stopping.load(Ordering::Acquire)
    }

    pub fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        self.request_shutdown();
        let entries = self
            .0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        let mut failure = None;
        for entry in entries {
            if let Err(error) = entry.terminate_until(deadline) {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests;
