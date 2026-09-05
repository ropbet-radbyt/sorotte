use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use sha2::{Digest, Sha256};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;

/// Pins validated directories and the canonical resource while mpv opens it.
/// The explicit-path convenience APIs return only a path: their callers must
/// preserve the validated store's ownership and permissions until use.
pub(crate) struct ResourceLease {
    path: PathBuf,
    file: fs::File,
    directories: platform::Directories,
}

impl ResourceLease {
    pub(crate) fn load_path(&self) -> io::Result<PathBuf> {
        platform::load_path(&self.path, &self.file, &self.directories)
    }
}

pub(crate) fn lease_bundled_sorotte_bridge() -> io::Result<ResourceLease> {
    lease_resource_in(
        &default_bridge_cache_root()?,
        SOROTTE_BRIDGE_FILE_NAME,
        BUNDLED_SOROTTE_BRIDGE,
    )
}

pub(crate) fn lease_bundled_sorotte_network_options_hook() -> io::Result<ResourceLease> {
    lease_resource_in(
        &default_bridge_cache_root()?,
        SOROTTE_NETWORK_OPTIONS_HOOK_FILE_NAME,
        BUNDLED_SOROTTE_NETWORK_OPTIONS_HOOK,
    )
}

const BUNDLED_SOROTTE_BRIDGE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/sorotte_syncplayintf.lua"
));
const BUNDLED_SOROTTE_NETWORK_OPTIONS_HOOK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/sorotte_network_options.lua"
));
const SOROTTE_BRIDGE_FILE_NAME: &str = "sorotte_syncplayintf.lua";
const SOROTTE_NETWORK_OPTIONS_HOOK_FILE_NAME: &str = "sorotte_network_options.lua";
static NEXT_TEMPORARY_RESOURCE: AtomicU64 = AtomicU64::new(1);

pub fn materialize_bundled_sorotte_bridge() -> io::Result<PathBuf> {
    publish_default_resource(SOROTTE_BRIDGE_FILE_NAME, BUNDLED_SOROTTE_BRIDGE)
}

/// Creates or reuses a private resource store. Existing roots and resources must
/// belong to the current user, exclude other users' access, and contain no links.
/// The caller owns the explicit root's lifetime and must keep it trusted until use.
pub fn materialize_bundled_sorotte_bridge_in(cache_root: &Path) -> io::Result<PathBuf> {
    materialize_bundled_resource_in(cache_root, SOROTTE_BRIDGE_FILE_NAME, BUNDLED_SOROTTE_BRIDGE)
}

pub fn materialize_bundled_sorotte_network_options_hook() -> io::Result<PathBuf> {
    publish_default_resource(
        SOROTTE_NETWORK_OPTIONS_HOOK_FILE_NAME,
        BUNDLED_SOROTTE_NETWORK_OPTIONS_HOOK,
    )
}

fn publish_default_resource(file_name: &str, content: &[u8]) -> io::Result<PathBuf> {
    // A path-only default API cannot tell when its caller finishes loading Lua.
    // Retain its two canonical resource leases for the process lifetime. Explicit
    // `_in` stores instead have the documented caller-owned lifetime contract.
    static PUBLISHED: std::sync::Mutex<Vec<ResourceLease>> = std::sync::Mutex::new(Vec::new());
    let mut published = PUBLISHED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(resource) = published
        .iter()
        .find(|resource| resource.path.file_name() == Some(std::ffi::OsStr::new(file_name)))
    {
        return Ok(resource.path.clone());
    }
    let resource = lease_resource_in(&default_bridge_cache_root()?, file_name, content)?;
    let path = resource.path.clone();
    published.push(resource);
    Ok(path)
}

/// Uses the same private-store contract as `materialize_bundled_sorotte_bridge_in`.
pub fn materialize_bundled_sorotte_network_options_hook_in(
    cache_root: &Path,
) -> io::Result<PathBuf> {
    materialize_bundled_resource_in(
        cache_root,
        SOROTTE_NETWORK_OPTIONS_HOOK_FILE_NAME,
        BUNDLED_SOROTTE_NETWORK_OPTIONS_HOOK,
    )
}

fn materialize_bundled_resource_in(
    cache_root: &Path,
    file_name: &str,
    content: &[u8],
) -> io::Result<PathBuf> {
    Ok(lease_resource_in(cache_root, file_name, content)?.path)
}

fn lease_resource_in(
    cache_root: &Path,
    file_name: &str,
    content: &[u8],
) -> io::Result<ResourceLease> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match try_lease_resource_in(cache_root, file_name, content) {
            Err(error)
                if ((cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33)))
                    || (cfg!(unix) && error.kind() == io::ErrorKind::NotFound))
                    && std::time::Instant::now() < deadline =>
            {
                // Readers keep published Lua immutable through load-script.
                // A concurrent repair may hold a short read lease on corrupt
                // bytes; Unix may unlink an already-open old inode while
                // replacing its canonical name. Windows also returns native
                // ACCESS_DENIED while a replaced file is delete-pending until
                // its last handle closes. Retry publication without
                // relaxing ownership, hard-link, or load-time identity checks.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result,
        }
    }
}

fn try_lease_resource_in(
    cache_root: &Path,
    file_name: &str,
    content: &[u8],
) -> io::Result<ResourceLease> {
    let content_hash = hex_sha256(content);
    let content_directory = cache_root.join(content_hash);
    let resource_path = content_directory.join(file_name);
    let directories = platform::prepare_directories(cache_root, &content_directory)?;
    if resource_matches(&resource_path, content)? {
        return finish_lease(resource_path, directories, content);
    }

    let temporary_path = content_directory.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY_RESOURCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut temporary = platform::create_private_file(&temporary_path)?;
    if let Err(error) = temporary
        .write_all(content)
        .and_then(|()| temporary.sync_all())
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    drop(temporary);

    if let Err(error) = replace_file_atomically(&temporary_path, &resource_path) {
        let _ = fs::remove_file(&temporary_path);
        if resource_matches(&resource_path, content)? {
            return finish_lease(resource_path, directories, content);
        }
        return Err(error);
    }

    finish_lease(resource_path, directories, content)
}

fn resource_matches(path: &Path, expected: &[u8]) -> io::Result<bool> {
    match platform::open_resource(path) {
        Ok(file) => reader_matches(file, expected),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn reader_matches(reader: impl Read, expected: &[u8]) -> io::Result<bool> {
    let mut content = Vec::with_capacity(expected.len() + 1);
    reader
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut content)?;
    Ok(content == expected)
}

fn finish_lease(
    path: PathBuf,
    directories: platform::Directories,
    content: &[u8],
) -> io::Result<ResourceLease> {
    let file = platform::open_resource(&path)?;
    if !reader_matches(&file, content)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bundled resource changed during publication",
        ));
    }
    Ok(ResourceLease {
        path,
        file,
        directories,
    })
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

    // SAFETY: Both NUL-terminated UTF-16 path buffers remain alive for the duration of this
    // call. Source and destination share one content-addressed cache directory, so Windows
    // performs an atomic replacement rather than a copy/delete operation.
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
    fs::rename(temporary_path, destination)
}

fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn default_bridge_cache_root() -> io::Result<PathBuf> {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    if let Some(root) = ROOT.get() {
        return Ok(root.clone());
    }
    let candidate = default_candidate(&|name| env::var_os(name));
    let root = if let Some(root) = candidate
        && platform::prepare_directories(&root, &root).is_ok()
    {
        root
    } else {
        // Never trust the old predictable temporary path. Exclusive creation
        // with native private permissions precedes publishing the random name.
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        let name = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = env::temp_dir().join(format!("sorotte-mpv-bridge-{name}"));
        platform::create_private_directory(&root)?;
        platform::prepare_directories(&root, &root)?;
        root
    };
    let _ = ROOT.set(root);
    Ok(ROOT.get().expect("resource root initialized").clone())
}

fn default_candidate(lookup: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    #[cfg(windows)]
    let candidate = lookup("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("Sorotte").join("cache").join("mpv-bridge"));
    #[cfg(unix)]
    let candidate = lookup("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .or_else(|| {
            lookup("HOME")
                .map(PathBuf::from)
                .filter(|root| root.is_absolute())
                .map(|root| root.join(".cache"))
        })
        .map(|root| root.join("sorotte").join("mpv-bridge"));
    candidate.filter(|root| root.is_absolute())
}

#[cfg(test)]
mod security_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_cache_root(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "sorotte-player-mpv-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_RESOURCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn bundled_bridge_materializes_under_hash_with_canonical_file_name() {
        let cache_root = unique_cache_root("resource-path");
        let path = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("bundled bridge should materialize");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(SOROTTE_BRIDGE_FILE_NAME)
        );
        assert_eq!(
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str()),
            Some(hex_sha256(BUNDLED_SOROTTE_BRIDGE).as_str())
        );
        assert_eq!(fs::read(&path).unwrap(), BUNDLED_SOROTTE_BRIDGE);

        let _ = fs::remove_dir_all(cache_root);
    }

    #[test]
    fn bundled_bridge_materialization_is_idempotent_and_repairs_corruption() {
        let cache_root = unique_cache_root("resource-repair");
        let first = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("first materialization should succeed");
        let second = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("second materialization should reuse the same path");
        assert_eq!(first, second);

        fs::write(&first, b"corrupt").unwrap();
        let repaired = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("corrupt materialization should be repaired");
        assert_eq!(repaired, first);
        assert_eq!(fs::read(&repaired).unwrap(), BUNDLED_SOROTTE_BRIDGE);

        let _ = fs::remove_dir_all(cache_root);
    }

    #[test]
    fn bundled_network_options_hook_materializes_independently_from_the_optional_bridge() {
        let cache_root = unique_cache_root("network-options-resource");
        let hook_path = materialize_bundled_sorotte_network_options_hook_in(&cache_root)
            .expect("bundled network-options hook should materialize");
        let bridge_path = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("bundled optional bridge should materialize");

        assert_eq!(
            hook_path.file_name().and_then(|name| name.to_str()),
            Some(SOROTTE_NETWORK_OPTIONS_HOOK_FILE_NAME)
        );
        assert_ne!(hook_path.parent(), bridge_path.parent());
        assert_eq!(
            fs::read(&hook_path).unwrap(),
            BUNDLED_SOROTTE_NETWORK_OPTIONS_HOOK
        );

        let _ = fs::remove_dir_all(cache_root);
    }

    #[test]
    fn concurrent_corrupt_materialization_repair_converges_on_the_embedded_resource() {
        const REPAIRERS: usize = 8;

        let cache_root = unique_cache_root("resource-concurrent-repair");
        let resource_path = materialize_bundled_sorotte_bridge_in(&cache_root)
            .expect("initial bridge materialization should succeed");
        fs::write(&resource_path, b"corrupt").unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(REPAIRERS));
        let repairers = (0..REPAIRERS)
            .map(|_| {
                let cache_root = cache_root.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let path = materialize_bundled_sorotte_bridge_in(&cache_root)
                        .expect("concurrent resource lease should succeed");
                    let content = fs::read(&path)
                        .expect("published canonical resource should remain readable");
                    if content != BUNDLED_SOROTTE_BRIDGE {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "concurrent repair returned a non-canonical bridge resource",
                        ));
                    }
                    Ok(path)
                })
            })
            .collect::<Vec<_>>();

        for repairer in repairers {
            assert_eq!(
                repairer
                    .join()
                    .expect("concurrent repair thread should not panic")
                    .expect("concurrent repair should tolerate canonical-file races"),
                resource_path
            );
        }
        assert_eq!(fs::read(&resource_path).unwrap(), BUNDLED_SOROTTE_BRIDGE);

        let _ = fs::remove_dir_all(cache_root);
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_delete_pending_resource_repair_waits_for_the_last_handle() {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use std::sync::mpsc;
        use std::time::Duration;
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_DISPOSITION_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            FileDispositionInfo, SetFileInformationByHandle,
        };

        let cache_root = unique_cache_root("resource-delete-pending");
        let path = materialize_bundled_sorotte_bridge_in(&cache_root).unwrap();
        let deleting = fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: The owned file handle and correctly sized native information
        // buffer remain valid. Legacy deletion deliberately stays pending
        // until this handle closes, unlike POSIX unlink semantics.
        let marked = unsafe {
            SetFileInformationByHandle(
                deleting.as_raw_handle(),
                FileDispositionInfo,
                &disposition as *const _ as *const _,
                std::mem::size_of_val(&disposition) as u32,
            )
        };
        assert_ne!(marked, 0);
        assert_eq!(
            platform::open_resource(&path).unwrap_err().raw_os_error(),
            Some(5)
        );

        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let repair_root = cache_root.clone();
        let repairer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(materialize_bundled_sorotte_bridge_in(&repair_root))
                .unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let pending = result_rx.recv_timeout(Duration::from_millis(100));
        drop(deleting);
        let repaired = match pending {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                result_rx.recv_timeout(Duration::from_secs(2)).unwrap()
            }
            result => {
                repairer.join().unwrap();
                panic!("repair must wait for deletion to finish: {result:?}");
            }
        };
        repairer.join().unwrap();
        assert_eq!(repaired.unwrap(), path);
        assert_eq!(fs::read(path).unwrap(), BUNDLED_SOROTTE_BRIDGE);
        fs::remove_dir_all(cache_root).unwrap();
    }
}
