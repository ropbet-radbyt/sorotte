use super::*;

#[test]
#[ignore = "isolated concurrent cache repair process entry point"]
fn cache_repair_process_fixture() {
    let cache = PathBuf::from(env::var_os("SOROTTE_CACHE_REPAIR_FIXTURE").unwrap());
    fs::write(
        cache.join(format!("ready-{}", std::process::id())),
        b"ready",
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cache.join("start-repair").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let path = materialize_bundled_sorotte_bridge_in(&cache).unwrap();
    assert_eq!(fs::read(path).unwrap(), BUNDLED_SOROTTE_BRIDGE);
}

#[test]
fn independent_processes_repair_one_corrupt_cache_without_partial_publication() {
    let cache = root("process-repair");
    let path = materialize_bundled_sorotte_bridge_in(&cache).unwrap();
    fs::write(&path, b"corrupt").unwrap();
    let mut children = Vec::new();
    for _ in 0..4 {
        let mut command =
            crate::managed_process::ManagedMpvCommand::new(env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bridge_resource::security_tests::cache_repair_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("SOROTTE_CACHE_REPAIR_FIXTURE", &cache);
        children.push(command.spawn(None).unwrap());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while children
        .iter()
        .any(|child| !cache.join(format!("ready-{}", child.id())).exists())
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    fs::write(cache.join("start-repair"), b"start").unwrap();
    for child in &mut children {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    drop(children);
    assert_eq!(fs::read(path).unwrap(), BUNDLED_SOROTTE_BRIDGE);
    fs::remove_dir_all(cache).unwrap();
}

fn root(label: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "sorotte-private-resource-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY_RESOURCE.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn resource_validation_reads_at_most_expected_bytes_plus_one() {
    struct Endless<'a>(&'a std::cell::Cell<usize>);
    impl Read for Endless<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            bytes.fill(b'x');
            self.0.set(self.0.get() + bytes.len());
            Ok(bytes.len())
        }
    }
    let count = std::cell::Cell::new(0);
    assert!(!reader_matches(Endless(&count), BUNDLED_SOROTTE_BRIDGE).unwrap());
    assert_eq!(count.get(), BUNDLED_SOROTTE_BRIDGE.len() + 1);
}

#[test]
fn oversized_cache_file_is_repaired_and_hard_links_are_rejected_without_touching_target() {
    let cache = root("oversize");
    let path = materialize_bundled_sorotte_bridge_in(&cache).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    assert_eq!(materialize_bundled_sorotte_bridge_in(&cache).unwrap(), path);
    assert_eq!(fs::read(&path).unwrap(), BUNDLED_SOROTTE_BRIDGE);
    let outside = root("outside-file");
    fs::hard_link(&path, &outside).unwrap();
    assert_eq!(
        materialize_bundled_sorotte_bridge_in(&cache)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read(&outside).unwrap(), BUNDLED_SOROTTE_BRIDGE);
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(cache).unwrap();
}

#[test]
fn materialize_to_load_seam_cannot_redirect_through_replaced_ancestor() {
    let cache = root("load-seam");
    let moved = root("moved");
    let outside = root("outside");
    platform::create_private_directory(&outside).unwrap();
    fs::write(
        outside.join(SOROTTE_BRIDGE_FILE_NAME),
        b"error('outside store')",
    )
    .unwrap();
    let lease =
        lease_resource_in(&cache, SOROTTE_BRIDGE_FILE_NAME, BUNDLED_SOROTTE_BRIDGE).unwrap();
    #[cfg(windows)]
    {
        assert!(
            fs::rename(&cache, &moved).is_err(),
            "the resource ancestry must stay pinned until load-script returns"
        );
        assert!(fs::write(&lease.path, b"changed after verification").is_err());
        assert!(fs::remove_file(&lease.path).is_err());
    }
    #[cfg(target_os = "linux")]
    {
        fs::rename(&cache, &moved).unwrap();
        platform::create_private_directory(&cache).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join(hex_sha256(BUNDLED_SOROTTE_BRIDGE)))
            .unwrap();
    }
    let load_path = lease.load_path().unwrap();
    assert_eq!(
        fs::read(load_path).unwrap(),
        BUNDLED_SOROTTE_BRIDGE,
        "load-script must still open verified bytes"
    );
    drop(lease);
    fs::remove_dir_all(cache).unwrap();
    if moved.exists() {
        fs::remove_dir_all(moved).unwrap();
    }
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn linked_cache_roots_hash_directories_and_ancestors_are_rejected() {
    for position in ["root", "hash", "ancestor"] {
        let cache = root(position);
        let outside = root("link-target");
        platform::create_private_directory(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"unchanged").unwrap();
        let requested = match position {
            "root" => {
                directory_link(&outside, &cache);
                cache.clone()
            }
            "hash" => {
                platform::create_private_directory(&cache).unwrap();
                directory_link(&outside, &cache.join(hex_sha256(BUNDLED_SOROTTE_BRIDGE)));
                cache.clone()
            }
            _ => {
                directory_link(&outside, &cache);
                cache.join("nested-store")
            }
        };
        assert!(
            materialize_bundled_sorotte_bridge_in(&requested).is_err(),
            "{position} link must fail closed"
        );
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"unchanged");
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            1,
            "linked target must not receive any cache writes"
        );
        // Remove the junction itself without traversing its destination.
        #[cfg(windows)]
        {
            if position == "hash" {
                fs::remove_dir(cache.join(hex_sha256(BUNDLED_SOROTTE_BRIDGE))).unwrap();
            } else {
                fs::remove_dir(&cache).unwrap();
            }
        }
        #[cfg(unix)]
        {
            if position == "hash" {
                fs::remove_file(cache.join(hex_sha256(BUNDLED_SOROTTE_BRIDGE))).unwrap();
            } else {
                fs::remove_file(&cache).unwrap();
            }
        }
        if cache.exists() {
            fs::remove_dir_all(cache).unwrap();
        }
        fs::remove_dir_all(outside).unwrap();
    }
}

#[cfg(unix)]
fn directory_link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn directory_link(target: &Path, link: &Path) {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::GENERIC_WRITE,
        Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
        System::IO::DeviceIoControl,
    };
    fs::create_dir(link).unwrap();
    let file = fs::OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(link)
        .unwrap();
    let target = target.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut substitute = r"\??\".encode_utf16().collect::<Vec<_>>();
    substitute.extend(&target);
    let substitute_bytes = (substitute.len() * 2) as u16;
    let print_bytes = (target.len() * 2) as u16;
    let mut payload = Vec::new();
    payload.extend(0xA0000003_u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    payload.extend((8 + substitute_bytes + print_bytes + 4).to_le_bytes());
    payload.extend(0_u16.to_le_bytes());
    payload.extend(0_u16.to_le_bytes());
    payload.extend(substitute_bytes.to_le_bytes());
    payload.extend((substitute_bytes + 2).to_le_bytes());
    payload.extend(print_bytes.to_le_bytes());
    for character in substitute
        .into_iter()
        .chain(Some(0))
        .chain(target)
        .chain(Some(0))
    {
        payload.extend(character.to_le_bytes());
    }
    let mut returned = 0;
    // SAFETY: the bounded fixture buffer follows REPARSE_DATA_BUFFER's mount-point
    // layout and the live handle identifies an empty fixture directory.
    assert_ne!(
        // SAFETY: the mount-point buffer and owned empty directory handle are valid.
        unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x0009_00a4,
                payload.as_ptr().cast(),
                payload.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0,
        "junction creation: {}",
        io::Error::last_os_error()
    );
}

#[cfg(unix)]
#[test]
fn non_private_roots_and_final_file_symlinks_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let cache = root("mode");
    platform::create_private_directory(&cache).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(materialize_bundled_sorotte_bridge_in(&cache).is_err());
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    let lease =
        lease_resource_in(&cache, SOROTTE_BRIDGE_FILE_NAME, BUNDLED_SOROTTE_BRIDGE).unwrap();
    let outside = root("symlink-outside");
    fs::write(&outside, b"unchanged").unwrap();
    fs::remove_file(&lease.path).unwrap();
    symlink(&outside, &lease.path).unwrap();
    assert!(lease.load_path().is_err());
    assert!(materialize_bundled_sorotte_bridge_in(&cache).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"unchanged");
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(cache).unwrap();
}

#[cfg(unix)]
#[test]
fn absent_or_relative_xdg_uses_private_home_cache_candidate() {
    for xdg in [None, Some("relative")] {
        assert_eq!(
            default_candidate(&|name| match name {
                "HOME" => Some("/home/test".into()),
                "XDG_CACHE_HOME" => xdg.map(Into::into),
                _ => None,
            }),
            Some(PathBuf::from("/home/test/.cache/sorotte/mpv-bridge"))
        );
    }
    assert!(default_candidate(&|_| None).is_none());
}
