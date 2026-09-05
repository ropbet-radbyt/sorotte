use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Component, Path, PathBuf},
};

pub(super) struct Directories {
    handles: Vec<File>,
}
fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)
}

pub(super) fn prepare_directories(root: &Path, content: &Path) -> io::Result<Directories> {
    if !root.is_absolute() || !content.starts_with(root) {
        return Err(denied("resource cache root must be absolute"));
    }
    // SAFETY: geteuid has no preconditions or side effects.
    let user = unsafe { libc::geteuid() };
    let mut path = PathBuf::new();
    let mut handles = Vec::new();
    for component in content.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(denied(
                "resource path must not traverse relative components",
            ));
        }
        path.push(component);
        let open = || {
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
        };
        let file = match open() {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_private_directory(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                open()?
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if path.starts_with(root) {
            if metadata.uid() != user || metadata.mode() & 0o077 != 0 {
                return Err(denied(
                    "resource store must be private and owned by the current user",
                ));
            }
        } else if metadata.uid() != user && metadata.uid() != 0 {
            return Err(denied("resource cache ancestor belongs to another user"));
        } else if metadata.mode() & 0o022 != 0
            && !(metadata.uid() == 0 && metadata.mode() & libc::S_ISVTX != 0)
        {
            return Err(denied(
                "resource cache ancestor is writable by another user",
            ));
        }
        handles.push(file);
    }
    Ok(Directories { handles })
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

pub(super) fn open_resource(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    validate_resource(&file)?;
    Ok(file)
}

fn validate_resource(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.nlink() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "bundled resource was replaced during validation",
        ));
    }
    // SAFETY: geteuid has no preconditions or side effects.
    let owner = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
    {
        return Err(denied(
            "bundled resource must be an ordinary private file owned by the current user",
        ));
    }
    Ok(())
}

pub(super) fn load_path(
    path: &Path,
    file: &File,
    directories: &Directories,
) -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let protected = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        directories
            .handles
            .last()
            .expect("content directory pinned")
            .as_raw_fd()
    ))
    .join(path.file_name().expect("canonical resource filename"));
    #[cfg(not(target_os = "linux"))]
    let protected = {
        let _ = directories.handles.len();
        path.to_path_buf()
    };
    let current = open_resource(&protected)?;
    let original = file.metadata()?;
    let current = current.metadata()?;
    if original.dev() != current.dev() || original.ino() != current.ino() {
        return Err(denied("bundled resource changed before load-script"));
    }
    Ok(protected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlinked_open_resource_is_retryable_but_hard_links_are_denied() {
        let root = std::env::temp_dir().join(format!(
            "sorotte-open-resource-race-{}-{}",
            std::process::id(),
            super::super::NEXT_TEMPORARY_RESOURCE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir(&root).unwrap();
        let canonical = root.join("bridge.lua");
        let alias = root.join("alias.lua");
        let old = create_private_file(&canonical).unwrap();
        validate_resource(&old).unwrap();
        std::fs::hard_link(&canonical, &alias).unwrap();
        assert_eq!(
            validate_resource(&old).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&canonical).unwrap();
        let replacement = create_private_file(&canonical).unwrap();
        validate_resource(&replacement).unwrap();
        assert_eq!(
            validate_resource(&old).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        drop((old, replacement));
        std::fs::remove_dir_all(root).unwrap();
    }
}
