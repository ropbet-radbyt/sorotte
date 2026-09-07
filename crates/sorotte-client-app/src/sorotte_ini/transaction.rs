use std::{
    ffi::{OsStr, OsString},
    fs::{File, TryLockError},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FILE_SYMLINKS: usize = 40;

#[cfg(test)]
thread_local! {
    pub(super) static CONTENTION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// A persistent sidecar is essential: locking the replaced settings file or
/// unlinking the sidecar after unlock would let clients lock different files.
pub(super) struct SettingsTransaction {
    lock: File,
    path: PathBuf,
}

impl SettingsTransaction {
    pub(super) fn acquire(path: &Path) -> anyhow::Result<Self> {
        Self::acquire_with_timeout(path, LOCK_TIMEOUT)
    }

    pub(super) fn acquire_with_timeout(path: &Path, timeout: Duration) -> anyhow::Result<Self> {
        let deadline = Instant::now() + timeout;
        let (lock, path) = prepare_writer_lock(path)?;
        lock_file(&lock, &path, false, deadline)?;
        validate_destination(&path)?;
        Ok(Self { lock, path })
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

/// Read-only acquisition never creates a directory or sidecar. If the first
/// writer creates a sidecar during a legacy/uninitialized read, its provisional
/// result (including an error or absence) is discarded and read under the lock.
pub(super) fn read_consistently<F>(path: &Path, read: F) -> anyhow::Result<Option<String>>
where
    F: FnMut(&Path) -> anyhow::Result<Option<String>>,
{
    read_consistently_with_timeout(path, LOCK_TIMEOUT, read)
}

pub(super) fn read_consistently_with_timeout<F>(
    path: &Path,
    timeout: Duration,
    mut read: F,
) -> anyhow::Result<Option<String>>
where
    F: FnMut(&Path) -> anyhow::Result<Option<String>>,
{
    let deadline = Instant::now() + timeout;
    let Some(path) = resolve_settings_path(path, false)? else {
        // Only a missing containing directory reaches this case. Cooperating
        // writers do not replace/remove directories, so absence is observed
        // before any later first writer creates that directory and its sidecar.
        return Ok(None);
    };
    if let Some(lock) = open_existing_lock(&path)? {
        lock_file(&lock, &path, true, deadline)?;
        return read(&path);
    }
    let provisional = read(&path);
    if let Some(lock) = open_existing_lock(&path)? {
        lock_file(&lock, &path, true, deadline)?;
        return read(&path);
    }
    provisional
}

fn lock_file(file: &File, path: &Path, shared: bool, deadline: Instant) -> anyhow::Result<()> {
    loop {
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match result {
            Ok(()) => return Ok(()),
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
                        "stored settings are busy; retry after the other transaction finishes: {}",
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

fn lock_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut name = OsString::from(".");
    name.push(
        path.file_name()
            .ok_or_else(|| anyhow!("settings path has no filename"))?,
    );
    name.push(".lock");
    Ok(path.with_file_name(name))
}

fn reject_linked_lock(path: &Path) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(anyhow!(
            "stored settings lock must not be a symbolic link: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed inspecting stored settings lock"),
    }
}

fn validate_lock(file: &File) -> anyhow::Result<()> {
    if !file.metadata()?.is_file() {
        return Err(anyhow!("stored settings lock is not a regular file"));
    }
    Ok(())
}

fn open_existing_lock(path: &Path) -> anyhow::Result<Option<File>> {
    let lock_path = lock_path(path)?;
    reject_linked_lock(&lock_path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        // Match writer sharing: keep the persistent sidecar from being unlinked
        // while its handle supplies the read lock. No write permission is needed.
        options
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    match options.open(&lock_path) {
        Ok(file) => {
            validate_lock(&file)?;
            Ok(Some(file))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("failed opening stored settings read lock"),
    }
}

fn prepare_writer_lock(path: &Path) -> anyhow::Result<(File, PathBuf)> {
    let path = resolve_settings_path(path, true)?
        .ok_or_else(|| anyhow!("stored settings parent is missing after creation"))?;
    let lock_path = lock_path(&path)?;
    reject_linked_lock(&lock_path)?;
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
    validate_lock(&file)?;
    let canonical_lock = std::fs::canonicalize(&lock_path)
        .context("failed resolving persistent stored settings lock")?;
    let name = canonical_lock
        .file_name()
        .and_then(|name| name.as_encoded_bytes().strip_prefix(b"."))
        .and_then(|name| name.strip_suffix(b".lock"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("stored settings lock has an invalid filename"))?;
    // SAFETY: This subslice comes from one OsStr's platform encoding, split
    // immediately after/before known ASCII characters. Those are valid encoding
    // boundaries even when the settings name contains non-Unicode OS strings.
    let name = unsafe { OsStr::from_encoded_bytes_unchecked(name) };
    Ok((file, canonical_lock.with_file_name(name)))
}

/// Writer preparation may create parents and the private persistent sidecar.
/// Canonicalizing that stable sidecar, rather than the replaced destination,
/// also gives relocation a consistent lock order for Windows case aliases.
pub(super) fn canonical_settings_path(path: &Path) -> anyhow::Result<PathBuf> {
    prepare_writer_lock(path).map(|(_, path)| path)
}

fn validate_destination(path: &Path) -> anyhow::Result<()> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(anyhow!(
            "stored settings path is not a file: {}",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed inspecting locked stored settings path"),
    }
}

fn resolve_settings_path(path: &Path, create_parent: bool) -> anyhow::Result<Option<PathBuf>> {
    let mut path = path.to_path_buf();
    for _ in 0..MAX_FILE_SYMLINKS {
        let name = path
            .file_name()
            .ok_or_else(|| anyhow!("settings path has no filename"))?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if create_parent {
            std::fs::create_dir_all(parent).context("failed creating stored settings directory")?;
        }
        let parent = match std::fs::canonicalize(parent) {
            Ok(parent) => parent,
            Err(error) if !create_parent && error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error).context("failed resolving stored settings directory"),
        };
        let candidate = parent.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                // Read the link itself even if its destination is temporarily
                // absent; falling back to the alias's name would split the lock.
                let target = std::fs::read_link(&candidate)
                    .context("failed resolving stored settings file symlink")?;
                path = parent.join(target);
            }
            Ok(_) => return Ok(Some(candidate)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Some(candidate)),
            Err(error) => return Err(error).context("failed resolving stored settings name"),
        }
    }
    Err(anyhow!("stored settings file symlink chain is too long"))
}
