use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr::null_mut,
    time::{SystemTime, UNIX_EPOCH},
};

use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_FILE_OBJECT,
            SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, READ_CONTROL, WRITE_DAC},
};

use super::{
    paths::{
        write_sorotte_ini_contents_atomically_with_injected_pre_commit,
        write_sorotte_ini_contents_atomically_with_injected_pre_write,
    },
    *,
};
use crate::legacy_settings::StoredClientSettingsMvp;

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sorotte-settings-acl-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        // Deliberately expose the fixture parent to Everyone. The product must
        // supply a descriptor at creation rather than relying on this directory.
        set_dacl(&path, "D:P(A;OICI;FA;;;WD)", true);
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("sorotte.ini")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn set_dacl(path: &Path, sddl: &str, protected: bool) {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = null_mut();
    // SAFETY: Fixture-only conversion with valid NUL-terminated SDDL and output pointers.
    unsafe {
        assert_ne!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut descriptor,
                null_mut()
            ),
            0
        );
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        assert_ne!(
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted),
            0
        );
        assert_eq!(
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION
                    | if protected {
                        PROTECTED_DACL_SECURITY_INFORMATION
                    } else {
                        UNPROTECTED_DACL_SECURITY_INFORMATION
                    },
                null_mut(),
                null_mut(),
                dacl,
                null_mut()
            ),
            0
        );
        LocalFree(descriptor);
    }
}

fn security(path: &Path) -> (String, bool) {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    security_of_file(&file)
}
fn security_of_file(file: &File) -> (String, bool) {
    let mut descriptor = null_mut();
    // SAFETY: Live handle and valid outputs; both API allocations are freed.
    unsafe {
        assert_eq!(
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor
            ),
            0
        );
        let mut control = 0;
        let mut revision = 0;
        assert_ne!(
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
            0
        );
        let mut text = null_mut();
        let mut length = 0;
        assert_ne!(
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                &mut text,
                &mut length
            ),
            0
        );
        let units = std::slice::from_raw_parts(text, length as usize);
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        // AUTO_INHERITED is bookkeeping, not an ACE or protection rule;
        // CreateFileW may clear it while preserving the exact protected DACL.
        let value = String::from_utf16_lossy(&units[..end]).replace("D:PAI", "D:P");
        LocalFree(text.cast());
        LocalFree(descriptor);
        (value, control & SE_DACL_PROTECTED != 0)
    }
}

fn assert_private(state: &(String, bool)) {
    assert!(state.1, "DACL must be protected");
    assert_eq!(
        state.0.matches('(').count(),
        1,
        "new private ACL has exactly one owner grant"
    );
    let owner = state
        .0
        .strip_prefix("O:")
        .unwrap()
        .split("D:")
        .next()
        .unwrap();
    assert!(
        state.0.ends_with(&format!(";;;{owner})")),
        "grant principal must be the owner"
    );
}

#[test]
fn windows_new_file_and_empty_temporary_file_are_private_under_permissive_parent() {
    let fixture = Fixture::new("creation");
    write_sorotte_ini_contents_atomically_with_injected_pre_write(
        &fixture.path(),
        b"synthetic-secret",
        |file, temporary| {
            assert_eq!(
                file.metadata()?.len(),
                0,
                "inspect permissions before writing any secret bytes"
            );
            assert_private(&security_of_file(file));
            assert_private(&security(temporary));
            Ok(())
        },
    )
    .unwrap();
    assert_private(&security(&fixture.path()));
    assert_eq!(std::fs::read(fixture.path()).unwrap(), b"synthetic-secret");
}

#[test]
fn windows_protected_owner_and_dacl_survive_replacement_and_fault_cleanup() {
    let fixture = Fixture::new("preservation");
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), b"before").unwrap();
    let initial = security(&fixture.path());
    let owner = initial
        .0
        .strip_prefix("O:")
        .unwrap()
        .split("D:")
        .next()
        .unwrap();
    // Preserve a stricter explicit deny rule, not just the default owner grant.
    set_dacl(
        &fixture.path(),
        &format!("D:P(D;;WD;;;WD)(A;;FA;;;{owner})"),
        true,
    );
    let original = security(&fixture.path());
    assert!(original.1);
    assert_eq!(original.0.matches('(').count(), 2);
    for before_write in [false, true] {
        let result = if before_write {
            write_sorotte_ini_contents_atomically_with_injected_pre_write(
                &fixture.path(),
                b"after",
                |file, _| {
                    assert_eq!(security_of_file(file), original);
                    Err(io::Error::other("injected pre-write failure"))
                },
            )
        } else {
            write_sorotte_ini_contents_atomically_with_injected_pre_commit(
                &fixture.path(),
                b"after",
                |temporary| {
                    assert_eq!(security(temporary), original);
                    Err(io::Error::other("injected pre-commit failure"))
                },
            )
        };
        assert!(result.is_err());
        assert_eq!(std::fs::read(fixture.path()).unwrap(), b"before");
        assert_eq!(security(&fixture.path()), original);
        assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "tmp")
        }));
    }
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), b"after").unwrap();
    assert_eq!(security(&fixture.path()), original);
}

#[test]
fn windows_inherited_acl_is_tightened_and_read_only_failure_preserves_bytes_and_acl() {
    let fixture = Fixture::new("inheritance");
    std::fs::write(fixture.path(), b"before").unwrap();
    assert!(!security(&fixture.path()).1);
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), b"after").unwrap();
    let original = security(&fixture.path());
    assert_private(&original);
    let original_permissions = std::fs::metadata(fixture.path()).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(fixture.path(), permissions.clone()).unwrap();
    let result = write_sorotte_ini_contents_atomically_at_path(&fixture.path(), b"unexpected");
    assert!(result.is_err());
    assert_eq!(std::fs::read(fixture.path()).unwrap(), b"after");
    assert_eq!(security(&fixture.path()), original);
    std::fs::set_permissions(fixture.path(), original_permissions).unwrap();
}

#[test]
fn windows_nested_new_settings_and_private_directories_use_protected_descriptors() {
    let fixture = Fixture::new("nested");
    let path = fixture.0.join("new").join("nested").join("sorotte.ini");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &path,
        &StoredClientSettingsMvp {
            plex_user_token: Some("synthetic-token".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_private(&security(&path));
    let private = fixture.0.join("private-stage");
    create_private_directory(&private).unwrap();
    assert_private(&security(&private));
    let original = security(&private);
    assert_eq!(
        create_private_directory(&private).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(security(&private), original);
    std::fs::write(private.join("payload"), b"executable fixture").unwrap();
    let child = security(&private.join("payload"));
    assert_eq!(child.0.matches('(').count(), 1);
}
