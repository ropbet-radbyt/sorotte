use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use sha2::{Digest, Sha256};

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
    materialize_bundled_sorotte_bridge_in(&default_bridge_cache_root())
}

pub fn materialize_bundled_sorotte_bridge_in(cache_root: &Path) -> io::Result<PathBuf> {
    materialize_bundled_resource_in(cache_root, SOROTTE_BRIDGE_FILE_NAME, BUNDLED_SOROTTE_BRIDGE)
}

pub fn materialize_bundled_sorotte_network_options_hook() -> io::Result<PathBuf> {
    materialize_bundled_sorotte_network_options_hook_in(&default_bridge_cache_root())
}

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
    let content_hash = hex_sha256(content);
    let content_directory = cache_root.join(content_hash);
    let resource_path = content_directory.join(file_name);
    if resource_matches(&resource_path, content)? {
        return Ok(resource_path);
    }

    fs::create_dir_all(&content_directory)?;
    let temporary_path = content_directory.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY_RESOURCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut temporary = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)?;
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
            return Ok(resource_path);
        }
        return Err(error);
    }

    Ok(resource_path)
}

fn resource_matches(path: &Path, expected: &[u8]) -> io::Result<bool> {
    match fs::read(path) {
        Ok(content) => Ok(content == expected),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
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

fn default_bridge_cache_root() -> PathBuf {
    #[cfg(windows)]
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Sorotte")
            .join("cache")
            .join("mpv-bridge");
    }

    #[cfg(not(windows))]
    if let Some(xdg_cache_home) = env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg_cache_home)
            .join("sorotte")
            .join("mpv-bridge");
    }

    env::temp_dir().join("sorotte").join("mpv-bridge")
}

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
                    let path = materialize_bundled_sorotte_bridge_in(&cache_root)?;
                    let content = fs::read(&path)?;
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
}
