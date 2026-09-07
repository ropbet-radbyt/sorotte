use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::anyhow;

use crate::legacy_settings::StoredClientSettingsMvp;

use super::parser::parse_sorotte_ini_stored_client_settings_mvp;
use super::writer::{
    upsert_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
};
use super::{merge::merge_settings_contents, transaction::SettingsTransaction};

const TEMPORARY_FILE_ALLOCATION_ATTEMPTS: usize = 128;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Create one new private directory without following or modifying existing
/// entries. The parent must already exist. Used for credential and executable
/// staging where securing a directory after creation would leave an exposure.
pub fn create_private_directory(_path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        super::windows_security::SecurityDescriptor::create_private_directory(_path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(_path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private directory security is not supported on this platform",
        ))
    }
}

pub fn load_sorotte_ini_stored_client_settings_mvp_from_path(
    path: &Path,
) -> anyhow::Result<Option<StoredClientSettingsMvp>> {
    read_sorotte_ini_contents_consistently_at_path(path).map(|contents| {
        contents.map(|contents| parse_sorotte_ini_stored_client_settings_mvp(&contents))
    })
}

pub(crate) fn read_sorotte_ini_contents_consistently_at_path(
    path: &Path,
) -> anyhow::Result<Option<String>> {
    super::transaction::read_consistently(path, read_contents_under_transaction)
}

// Only used while holding a transaction, or by read_consistently for a
// provisional no-sidecar read whose result is checked against sidecar creation.
fn read_contents_under_transaction(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::Error::new(error)
            .context(format!("failed reading stored settings {}", path.display()))),
    }
}

/// Read the latest bytes and invoke the update exactly once under the writer's
/// transaction. Callback failures leave the existing document unchanged.
pub(crate) fn update_sorotte_ini_contents_at_path<F>(path: &Path, update: F) -> anyhow::Result<()>
where
    F: FnOnce(Option<&str>) -> anyhow::Result<String>,
{
    let transaction = SettingsTransaction::acquire(path)?;
    let contents = read_contents_under_transaction(transaction.path())?;
    let updated = update(contents.as_deref())?;
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(
        transaction.path(),
        updated.as_bytes(),
        |_| Ok(()),
    )
}

/// Preserve existing documents without requiring a writable directory or
/// creating a sidecar. A missing document is rechecked under the writer's lock.
pub(crate) fn ensure_sorotte_ini_contents_at_path(
    path: &Path,
    contents: &[u8],
) -> anyhow::Result<bool> {
    if read_sorotte_ini_contents_consistently_at_path(path)?.is_some() {
        return Ok(false);
    }
    let transaction = SettingsTransaction::acquire(path)?;
    if read_contents_under_transaction(transaction.path())?.is_some() {
        return Ok(false);
    }
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(
        transaction.path(),
        contents,
        |_| Ok(()),
    )?;
    Ok(true)
}

/// Apply an explicit field patch: `Some` assigns a field; `None` leaves it alone.
/// For an edited snapshot, use `merge_sorotte_ini_stored_client_settings_mvp_at_path`
/// with its original baseline to avoid restoring unrelated stale values.
pub fn upsert_sorotte_ini_stored_client_settings_mvp_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
        path,
        settings,
        upsert_sorotte_ini_stored_client_settings_mvp,
    )
}

pub fn upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
        path,
        settings,
        upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
    )
}

fn upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
    path: &Path,
    settings: &StoredClientSettingsMvp,
    writer: fn(&str, &StoredClientSettingsMvp) -> String,
) -> anyhow::Result<()> {
    let transaction = SettingsTransaction::acquire(path)?;
    let path = transaction.path();
    let existing_contents = read_contents_under_transaction(path)?.unwrap_or_default();
    let updated_contents = writer(&existing_contents, settings);
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(
        path,
        updated_contents.as_bytes(),
        |_| Ok(()),
    )
}

/// Unconditional byte replacement, serialized with all settings transactions.
/// Callers holding an edited settings snapshot must use the merge API instead.
pub fn write_sorotte_ini_contents_atomically_at_path(
    path: &Path,
    contents: &[u8],
) -> anyhow::Result<()> {
    let transaction = SettingsTransaction::acquire(path)?;
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(transaction.path(), contents, |_| {
        Ok(())
    })
}

fn write_sorotte_ini_contents_atomically_with_pre_commit_hook<F>(
    path: &Path,
    contents: &[u8],
    before_replace: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    write_sorotte_ini_contents_atomically_with_hooks(path, contents, |_, _| Ok(()), before_replace)
}

fn write_sorotte_ini_contents_atomically_with_hooks<B, F>(
    path: &Path,
    contents: &[u8],
    before_write: B,
    before_replace: F,
) -> anyhow::Result<()>
where
    B: FnOnce(&File, &Path) -> io::Result<()>,
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    #[cfg(windows)]
    {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.permissions().readonly() => {
                return Err(anyhow!(
                    "stored settings destination is read-only: {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let (temporary_path, temporary_file) = create_unique_temporary_file(parent, path)?;
    let mut temporary_file_guard = TemporaryFileGuard::new(temporary_path, temporary_file);
    before_write(
        temporary_file_guard
            .file
            .as_ref()
            .expect("temporary file is open"),
        temporary_file_guard.path(),
    )?;
    write_and_sync_temporary_file(
        temporary_file_guard
            .file
            .take()
            .expect("temporary file is open"),
        temporary_file_guard.path(),
        contents,
    )?;
    before_replace(temporary_file_guard.path()).map_err(|error| {
        anyhow!(
            "stored settings write stopped before replacing {}: {error}",
            path.display()
        )
    })?;
    replace_file_atomically(temporary_file_guard.path(), path).map_err(|error| {
        anyhow!(
            "failed replacing stored settings {} atomically: {error}",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
pub(super) fn write_sorotte_ini_contents_atomically_with_injected_pre_write<F>(
    path: &Path,
    contents: &[u8],
    before_write: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&File, &Path) -> io::Result<()>,
{
    write_sorotte_ini_contents_atomically_with_hooks(path, contents, before_write, |_| Ok(()))
}

fn create_unique_temporary_file(
    parent: &Path,
    destination: &Path,
) -> anyhow::Result<(PathBuf, File)> {
    #[cfg(windows)]
    let security = super::windows_security::SecurityDescriptor::for_destination(destination)
        .map_err(|error| anyhow!("failed preparing stored settings security: {error}"))?;
    for _ in 0..TEMPORARY_FILE_ALLOCATION_ATTEMPTS {
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut temporary_file_name = OsString::from(".");
        temporary_file_name.push(
            destination
                .file_name()
                .unwrap_or_else(|| OsStr::new("sorotte.ini")),
        );
        temporary_file_name.push(format!(
            ".{}.{}.{sequence}.tmp",
            std::process::id(),
            timestamp
        ));
        let temporary_path = parent.join(temporary_file_name);

        #[cfg(windows)]
        let created = security.create_file(&temporary_path, true);
        #[cfg(not(windows))]
        let created = {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&temporary_path)
        };
        match created {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(anyhow!(
                    "failed creating temporary stored settings file {}: {error}",
                    temporary_path.display()
                ));
            }
        }
    }

    Err(anyhow!(
        "failed allocating a unique temporary stored settings file in {}",
        parent.display()
    ))
}

fn write_and_sync_temporary_file(
    mut file: File,
    path: &Path,
    contents: &[u8],
) -> anyhow::Result<()> {
    // Creation already grants only the permitted principals. Tighten the Unix
    // mode using the open handle before any secret is written, independently of umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents).map_err(|error| {
        anyhow!(
            "failed writing temporary stored settings file {}: {error}",
            path.display()
        )
    })?;
    file.flush().map_err(|error| {
        anyhow!(
            "failed flushing temporary stored settings file {}: {error}",
            path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        anyhow!(
            "failed syncing temporary stored settings file {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

struct TemporaryFileGuard {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf, file: File) -> Self {
        Self {
            path,
            file: Some(file),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn replace_file_atomically(temporary_path: &Path, destination: &Path) -> io::Result<()> {
    // The pinned Rust implementation handles replacement with an open reader.
    // Cooperating settings readers take the persistent sidecar's shared lock;
    // this namespace operation does not promise uninterrupted external opens.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        let Err(error) = std::fs::rename(temporary_path, destination) else {
            return Ok(());
        };
        // Retry the rename/source-open operation, never the FnOnce update or
        // secret write. A scanner may transiently deny delete sharing.
        if matches!(error.raw_os_error(), Some(5 | 32)) && std::time::Instant::now() < deadline {
            std::thread::sleep(
                std::time::Duration::from_millis(1)
                    .min(deadline.saturating_duration_since(std::time::Instant::now())),
            );
        } else {
            return Err(error);
        }
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(temporary_path: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(temporary_path, destination)
}

#[cfg(test)]
pub(super) fn write_sorotte_ini_contents_atomically_with_injected_pre_commit<F>(
    path: &Path,
    contents: &[u8],
    before_replace: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(path, contents, before_replace)
}

pub fn update_sorotte_ini_stored_client_settings_mvp_at_path<F>(
    path: &Path,
    update: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&mut StoredClientSettingsMvp),
{
    edit_sorotte_ini_stored_client_settings_mvp_at_path(path, update).map(|_| ())
}

pub fn clear_sorotte_ini_stored_client_settings_mvp_at_path(path: &Path) -> anyhow::Result<bool> {
    let transaction = SettingsTransaction::acquire(path)?;
    let path = transaction.path();
    transaction.mark_cleared()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(anyhow!(
            "failed clearing stored settings {}: {error}",
            path.display()
        )),
    }
}

/// Invoke `update` exactly once while holding the path's transaction lock.
/// Assigning `None` removes a recognized field, including all duplicate secrets.
/// Returns the actual committed settings for the caller's next baseline.
pub fn edit_sorotte_ini_stored_client_settings_mvp_at_path<F>(
    path: &Path,
    update: F,
) -> anyhow::Result<StoredClientSettingsMvp>
where
    F: FnOnce(&mut StoredClientSettingsMvp),
{
    let transaction = SettingsTransaction::acquire(path)?;
    let path = transaction.path();
    let contents = read_contents_under_transaction(path)?.unwrap_or_default();
    let baseline = parse_sorotte_ini_stored_client_settings_mvp(&contents);
    let mut settings = baseline.clone();
    update(&mut settings);
    commit_merged_settings(path, &contents, &baseline, &settings)
}

/// Merge fields changed from `baseline`, preserving all other current disk fields.
/// Concurrent edits to the same field follow commit order (last transaction wins).
/// An initial missing file saves the whole snapshot; a durable Clear tombstone
/// prevents this initialization fallback after a file was deliberately cleared.
pub fn merge_sorotte_ini_stored_client_settings_mvp_at_path(
    path: &Path,
    baseline: &StoredClientSettingsMvp,
    desired: &StoredClientSettingsMvp,
) -> anyhow::Result<StoredClientSettingsMvp> {
    let transaction = SettingsTransaction::acquire(path)?;
    let path = transaction.path();
    let contents = read_contents_under_transaction(path)?;
    let initial = StoredClientSettingsMvp::default();
    let baseline = if contents.is_none() && !transaction.was_cleared()? {
        &initial
    } else {
        baseline
    };
    commit_merged_settings(
        path,
        contents.as_deref().unwrap_or_default(),
        baseline,
        desired,
    )
}

fn commit_merged_settings(
    path: &Path,
    contents: &str,
    baseline: &StoredClientSettingsMvp,
    desired: &StoredClientSettingsMvp,
) -> anyhow::Result<StoredClientSettingsMvp> {
    let updated = merge_settings_contents(contents, baseline, desired);
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(path, updated.as_bytes(), |_| {
        Ok(())
    })?;
    Ok(parse_sorotte_ini_stored_client_settings_mvp(&updated))
}

/// Relocate a snapshot while locking source and destination in canonical order.
/// `publish_location` runs once with both locks held. A publication failure rolls
/// back the destination before another settings writer can observe its commit.
pub fn relocate_sorotte_ini_stored_client_settings_mvp_at_path<F>(
    source: Option<&Path>,
    destination: &Path,
    baseline: &StoredClientSettingsMvp,
    desired: &StoredClientSettingsMvp,
    publish_location: F,
) -> anyhow::Result<StoredClientSettingsMvp>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    use super::transaction::canonical_settings_path;
    let destination = canonical_settings_path(destination)?;
    let source = source.map(canonical_settings_path).transpose()?;
    let mut paths = vec![destination.clone()];
    if let Some(source) = &source {
        paths.push(source.clone());
    }
    paths.sort();
    paths.dedup();
    let transactions = paths
        .iter()
        .map(|path| SettingsTransaction::acquire(path))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let previous = read_contents_under_transaction(&destination)?;
    let source_contents = match source.as_deref() {
        Some(source) => read_contents_under_transaction(source)?,
        None => None,
    };
    let source_was_cleared = source
        .as_deref()
        .and_then(|source| {
            transactions
                .iter()
                .find(|transaction| transaction.path() == source)
        })
        .map(|transaction| transaction.was_cleared())
        .transpose()?
        .unwrap_or(false);
    let initial = StoredClientSettingsMvp::default();
    let baseline = if source_contents.is_none() && !source_was_cleared {
        &initial
    } else {
        baseline
    };
    let settings = parse_sorotte_ini_stored_client_settings_mvp(&merge_settings_contents(
        source_contents.as_deref().unwrap_or_default(),
        baseline,
        desired,
    ));
    let previous_settings =
        parse_sorotte_ini_stored_client_settings_mvp(previous.as_deref().unwrap_or_default());
    let committed = commit_merged_settings(
        &destination,
        previous.as_deref().unwrap_or_default(),
        &previous_settings,
        &settings,
    )?;
    if let Err(error) = publish_location() {
        let rollback = match previous {
            Some(contents) => write_sorotte_ini_contents_atomically_with_pre_commit_hook(
                &destination,
                contents.as_bytes(),
                |_| Ok(()),
            ),
            None => std::fs::remove_file(&destination).map_err(anyhow::Error::from),
        };
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback) => Err(anyhow!(
                "{error}; restoring the configuration destination also failed: {rollback}"
            )),
        };
    }
    Ok(committed)
}
