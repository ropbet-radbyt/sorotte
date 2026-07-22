use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions, Permissions},
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

const TEMPORARY_FILE_ALLOCATION_ATTEMPTS: usize = 128;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn load_sorotte_ini_stored_client_settings_mvp_from_path(
    path: &Path,
) -> anyhow::Result<Option<StoredClientSettingsMvp>> {
    if !path.is_file() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("failed reading stored settings {}: {error}", path.display()))?;
    Ok(Some(parse_sorotte_ini_stored_client_settings_mvp(
        &contents,
    )))
}

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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            anyhow!(
                "failed creating stored settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let existing_contents = if path.is_file() {
        std::fs::read_to_string(path).map_err(|error| {
            anyhow!("failed reading stored settings {}: {error}", path.display())
        })?
    } else {
        String::new()
    };
    let updated_contents = writer(&existing_contents, settings);
    write_sorotte_ini_contents_atomically_at_path(path, updated_contents.as_bytes())
}

pub fn write_sorotte_ini_contents_atomically_at_path(
    path: &Path,
    contents: &[u8],
) -> anyhow::Result<()> {
    write_sorotte_ini_contents_atomically_with_pre_commit_hook(path, contents, |_| Ok(()))
}

fn write_sorotte_ini_contents_atomically_with_pre_commit_hook<F>(
    path: &Path,
    contents: &[u8],
    before_replace: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let preserved_permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let (temporary_path, temporary_file) = create_unique_temporary_file(parent, path.file_name())?;
    let temporary_file_guard = TemporaryFileGuard::new(temporary_path);

    write_and_sync_temporary_file(
        temporary_file,
        temporary_file_guard.path(),
        contents,
        preserved_permissions,
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
    enforce_owner_only_permissions(path)
}

fn create_unique_temporary_file(
    parent: &Path,
    destination_file_name: Option<&OsStr>,
) -> anyhow::Result<(PathBuf, File)> {
    for _ in 0..TEMPORARY_FILE_ALLOCATION_ATTEMPTS {
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut temporary_file_name = OsString::from(".");
        temporary_file_name
            .push(destination_file_name.unwrap_or_else(|| OsStr::new("sorotte.ini")));
        temporary_file_name.push(format!(
            ".{}.{}.{sequence}.tmp",
            std::process::id(),
            timestamp
        ));
        let temporary_path = parent.join(temporary_file_name);

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(&temporary_path) {
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
    preserved_permissions: Option<Permissions>,
) -> anyhow::Result<()> {
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
    drop(file);

    #[cfg(windows)]
    if let Some(permissions) = preserved_permissions {
        std::fs::set_permissions(path, permissions).map_err(|error| {
            anyhow!(
                "failed preserving stored settings permissions on {}: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(windows))]
    let _ = preserved_permissions;
    enforce_owner_only_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn enforce_owner_only_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, Permissions::from_mode(0o600)).map_err(|error| {
        anyhow!(
            "failed securing stored settings permissions on {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn enforce_owner_only_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

struct TemporaryFileGuard {
    path: PathBuf,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn replace_file_atomically(temporary_path: &Path, destination: &Path) -> io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temporary_wide = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();

    // SAFETY: Both NUL-terminated UTF-16 path buffers remain alive for the
    // duration of this call. The source and destination share one directory,
    // so Windows performs a rename rather than a copy/delete operation.
    let moved = unsafe {
        MoveFileExW(
            temporary_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
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
    let mut settings =
        load_sorotte_ini_stored_client_settings_mvp_from_path(path)?.unwrap_or_default();
    update(&mut settings);
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(path, &settings)
}

pub fn clear_sorotte_ini_stored_client_settings_mvp_at_path(path: &Path) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(anyhow!(
            "stored settings path is not a file and cannot be cleared: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path).map_err(|error| {
        anyhow!(
            "failed clearing stored settings {}: {error}",
            path.display()
        )
    })?;
    Ok(true)
}
