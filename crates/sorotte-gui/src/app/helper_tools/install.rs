use super::{MAX_EXECUTABLE_BYTES, check_cancelled};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

fn regular_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Helper directory is a reparse point.".to_owned());
        }
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Helper path is not a regular directory.".to_owned());
    }
    Ok(())
}

pub(in crate::app) struct ToolInstall {
    bin: PathBuf,
    stage: PathBuf,
    _lock: fs::File,
    preserve_stage: bool,
}

impl ToolInstall {
    pub(in crate::app) fn begin(bin: &Path, cancel: Option<&AtomicBool>) -> Result<Self, String> {
        check_cancelled(cancel)?;
        fs::create_dir_all(bin).map_err(|error| error.to_string())?;
        regular_directory(bin)?;
        let bin = bin.canonicalize().map_err(|error| error.to_string())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(bin.join(".install.lock"))
            .map_err(|error| error.to_string())?;
        lock.try_lock()
            .map_err(|error| format!("Cannot lock helper installation: {error}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let stage = bin.join(format!(".install-stage-{}-{nonce}", std::process::id()));
        sorotte_client_app::app_boundary::persistence::create_private_directory(&stage)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            bin,
            stage,
            _lock: lock,
            preserve_stage: false,
        })
    }

    pub(in crate::app) fn path(&self, file_name: &str) -> PathBuf {
        assert_eq!(
            Path::new(file_name).file_name(),
            Some(std::ffi::OsStr::new(file_name))
        );
        self.stage.join(file_name)
    }

    pub(in crate::app) fn copy_executable(
        &self,
        source: &Path,
        name: &str,
        cancel: Option<&AtomicBool>,
    ) -> Result<PathBuf, String> {
        let source = fs::File::open(source)
            .map_err(|error| format!("Cannot open helper import: {error}"))?;
        let target = self.path(name);
        copy_executable(source, &target, cancel)?;
        Ok(target)
    }

    pub(in crate::app) fn write_metadata(
        &self,
        metadata: &impl serde::Serialize,
    ) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(metadata).map_err(|error| error.to_string())?;
        let mut file =
            fs::File::create(self.path("metadata.json")).map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())
    }

    pub(in crate::app) fn commit(
        mut self,
        names: &[&str],
        cancel: Option<&AtomicBool>,
    ) -> Result<(), String> {
        check_cancelled(cancel)?;
        let backup = self.stage.join("backup");
        fs::create_dir(&backup).map_err(|error| error.to_string())?;
        let mut replaced = Vec::new();
        let result = (|| {
            for name in names
                .iter()
                .copied()
                .chain(std::iter::once("metadata.json"))
            {
                check_cancelled(cancel)?;
                let source = self.path(name);
                let target = self.bin.join(name);
                if !fs::symlink_metadata(&source)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                {
                    return Err(format!(
                        "Staged helper component is not a regular file: {name}"
                    ));
                }
                let had_previous = match fs::symlink_metadata(&target) {
                    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                        fs::rename(&target, backup.join(name))
                            .map_err(|error| error.to_string())?;
                        true
                    }
                    Ok(_) => {
                        return Err(format!(
                            "Installed helper component is not a regular file: {name}"
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => return Err(error.to_string()),
                };
                replaced.push((name, had_previous));
                fs::rename(source, target).map_err(|error| error.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (name, had_previous) in replaced.into_iter().rev() {
                let target = self.bin.join(name);
                if let Err(error) = fs::remove_file(&target)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    rollback_errors.push(error.to_string());
                    continue;
                }
                if had_previous && let Err(error) = fs::rename(backup.join(name), target) {
                    rollback_errors.push(error.to_string());
                }
            }
            if !rollback_errors.is_empty() {
                self.preserve_stage = true;
                return Err(format!(
                    "{error}; rollback incomplete: {}; retained backups at {}",
                    rollback_errors.join("; "),
                    backup.display()
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for ToolInstall {
    fn drop(&mut self) {
        if !self.preserve_stage {
            let _ = fs::remove_dir_all(&self.stage);
        }
    }
}

fn copy_executable(
    mut reader: impl Read,
    target: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    let mut file = fs::File::create(target).map_err(|error| error.to_string())?;
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_cancelled(cancel)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > MAX_EXECUTABLE_BYTES {
            return Err("Helper executable exceeds the byte limit.".to_owned());
        }
        file.write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(in crate::app) fn extract_executable(
    archive: &Path,
    file_name: &str,
    target: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if zip.len() > 10_000 {
        return Err("Helper archive has too many entries.".to_owned());
    }
    let mut selected = None;
    for index in 0..zip.len() {
        check_cancelled(cancel)?;
        let entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let normalized = entry.name().replace('\\', "/");
        if normalized.rsplit('/').next() != Some(file_name) {
            continue;
        }
        if selected.replace(index).is_some() {
            return Err(format!(
                "Helper archive contains duplicate {file_name} entries."
            ));
        }
        if !entry.is_file()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            || entry.size() > MAX_EXECUTABLE_BYTES
        {
            return Err(format!(
                "Helper archive contains an invalid {file_name} entry."
            ));
        }
    }
    let index = selected.ok_or_else(|| format!("Helper archive did not contain {file_name}."))?;
    copy_executable(
        zip.by_index(index).map_err(|error| error.to_string())?,
        target,
        cancel,
    )
}
