use std::{
    ffi::OsString,
    fs::{File, TryLockError},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
thread_local! {
    pub(super) static CONTENTION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// A persistent sidecar is essential: locking the replaced settings file or
/// unlinking the sidecar after unlock would let two writers lock different files.
pub(super) struct SettingsTransaction {
    lock: File,
    path: PathBuf,
}

impl SettingsTransaction {
    pub(super) fn acquire(path: &Path) -> anyhow::Result<Self> {
        Self::acquire_with_timeout(path, LOCK_TIMEOUT)
    }

    pub(super) fn acquire_with_timeout(path: &Path, timeout: Duration) -> anyhow::Result<Self> {
        let path = canonical_settings_path(path)?;
        let mut lock_name = OsString::from(".");
        lock_name.push(
            path.file_name()
                .ok_or_else(|| anyhow!("settings path has no filename"))?,
        );
        lock_name.push(".lock");
        let lock_path = path.with_file_name(lock_name);
        if std::fs::symlink_metadata(&lock_path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(anyhow!(
                "stored settings lock must not be a symbolic link: {}",
                lock_path.display()
            ));
        }
        #[cfg(windows)]
        let file = super::windows_security::SecurityDescriptor::owner_only()?
            .create_file(&lock_path, false)?;
        #[cfg(not(windows))]
        let file = {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&lock_path)?
        };
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { lock: file, path }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    #[cfg(test)]
                    CONTENTION_HOOK.with(|hook| {
                        if let Some(hook) = hook.borrow_mut().take() {
                            hook();
                        }
                    });
                    std::thread::sleep(
                        Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!(
                            "stored settings are busy; retry after the other writer finishes: {}",
                            path.display()
                        ),
                    )
                    .into());
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).context("failed locking stored settings");
                }
            }
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn was_cleared(&self) -> io::Result<bool> {
        Ok(self.lock.metadata()?.len() != 0)
    }

    // Keep a durable, nonsecret tombstone before deleting the settings file. A
    // missing file after Clear is distinct from first-run initialization; a
    // stale first-run/full snapshot must not repopulate cleared credentials.
    pub(super) fn mark_cleared(&self) -> io::Result<()> {
        self.lock.set_len(1)?;
        self.lock.sync_all()
    }
}

pub(super) fn canonical_settings_path(path: &Path) -> anyhow::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) if path.is_file() => return Ok(path),
        Ok(_) => {
            return Err(anyhow!(
                "stored settings path is not a file: {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("failed resolving stored settings path"),
    }
    // Resolve the containing directory even before the first file is created.
    // This unifies relative paths, '..', junctions, and directory symlinks.
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).context("failed creating stored settings directory")?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("settings path has no filename"))?;
    Ok(std::fs::canonicalize(parent)?.join(name))
}
