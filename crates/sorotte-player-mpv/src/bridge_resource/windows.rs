//! Private ACLs are applied at creation. Directory handles deny deletion from
//! the volume root through the resource, so an ancestor writable by another
//! account cannot redirect the pathname between validation and mpv's open.
use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileInformationByHandle, READ_CONTROL,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) struct Directories {
    _handles: Vec<File>,
}

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: descriptor constructors return LocalAlloc-owned memory.
        unsafe {
            LocalFree(self.0);
        }
    }
}
impl Descriptor {
    fn private() -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: valid process pseudo-handle and writable token pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call returned an owned token handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut length = 0;
        // SAFETY: null output queries the required token buffer size.
        unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut length);
        }
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; (length as usize).div_ceil(size_of::<usize>())];
        // SAFETY: buffer size and alignment satisfy TOKEN_USER and the embedded SID.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful query initialized TOKEN_USER in live storage.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = null_mut();
        // SAFETY: SID is inside live token storage and output is writable.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: conversion returned a terminated UTF-16 LocalAlloc string.
        let sid_text = unsafe {
            let mut length = 0;
            while *sid.add(length) != 0 {
                length += 1;
            }
            let result = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
            LocalFree(sid.cast());
            result
        };
        let sddl = format!("O:{sid_text}D:P(A;OICI;FA;;;{sid_text})")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut result = null_mut();
        // SAFETY: terminated SDDL remains live and result receives an owned descriptor.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut result,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(result))
    }

    fn from_file(file: &File) -> io::Result<Self> {
        let mut descriptor = null_mut();
        // SAFETY: live file and correctly typed writable descriptor pointer.
        let error = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(Self(descriptor))
    }

    fn verify_private(&self, directory: bool) -> io::Result<()> {
        let expected = Self::private()?;
        let (mut owner, mut user, mut defaulted, mut present, mut acl) =
            (null_mut(), null_mut(), 0, 0, null_mut());
        let (mut control, mut revision) = (0, 0);
        // SAFETY: both descriptors are valid and every out pointer remains live.
        let valid = unsafe {
            GetSecurityDescriptorOwner(self.0, &mut owner, &mut defaulted) != 0
                && GetSecurityDescriptorOwner(expected.0, &mut user, &mut defaulted) != 0
                && GetSecurityDescriptorDacl(self.0, &mut present, &mut acl, &mut defaulted) != 0
                && GetSecurityDescriptorControl(self.0, &mut control, &mut revision) != 0
        };
        if !valid {
            return Err(io::Error::last_os_error());
        }
        if owner.is_null() || user.is_null() || present == 0 || acl.is_null() {
            return Err(denied(
                "resource store has no verifiable owner or private DACL",
            ));
        }
        // SAFETY: owner and user point to SIDs inside live descriptors.
        if unsafe { EqualSid(owner, user) } == 0 || (directory && control & SE_DACL_PROTECTED == 0)
        {
            return Err(denied(
                "resource store is not protected and owned by the current user",
            ));
        }
        // SAFETY: the descriptor API returned a valid ACL.
        let count = unsafe { (*acl).AceCount };
        for index in 0..count {
            let mut ace = null_mut();
            // SAFETY: index is below the ACL's declared ACE count.
            if unsafe { GetAce(acl, index as u32, &mut ace) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: all ACEs begin with ACE_HEADER; only ordinary allow ACEs are accepted.
            let header = unsafe { &*ace.cast::<windows_sys::Win32::Security::ACE_HEADER>() };
            if header.AceType != 0 || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Err(denied("resource store contains an unsupported access rule"));
            }
            // SAFETY: verified ACE kind and size expose the SID beginning at SidStart.
            let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
            let allowed_sid = (&allowed.SidStart as *const u32).cast_mut().cast();
            // SAFETY: allowed SID and token user SID belong to valid API-provided ACEs.
            if unsafe { EqualSid(allowed_sid, user) } == 0 {
                return Err(denied("resource store grants access to another account"));
            }
        }
        Ok(())
    }
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut result = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if result.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "resource path contains NUL",
        ));
    }
    result.push(0);
    Ok(result)
}

pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    let descriptor = Descriptor::private()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path = wide(path)?;
    // SAFETY: terminated path and private descriptor remain live throughout creation.
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn prepare_directories(root: &Path, content: &Path) -> io::Result<Directories> {
    if !root.is_absolute() || !content.starts_with(root) {
        return Err(denied("resource cache root must be absolute"));
    }
    let mut handles = Vec::new();
    let mut path = PathBuf::new();
    for component in content.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(denied(
                    "resource path must not traverse relative components",
                ));
            }
            Component::Prefix(prefix) => {
                if !matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ) {
                    return Err(denied("resource cache must use a local volume"));
                }
                path.push(component);
                continue;
            }
            _ => path.push(component),
        }
        let open = || {
            OpenOptions::new()
                .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
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
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(denied(
                "resource path contains a reparse point or non-directory",
            ));
        }
        let security = Descriptor::from_file(&file)?;
        if path.starts_with(root) {
            security.verify_private(true)?;
        }
        // Ancestor descriptors have been read through these exact handles. We
        // need not trust their owners: the non-delete sharing denies replacement
        // until after mpv opens the verified private descendant.
        handles.push(file);
    }
    Ok(Directories { _handles: handles })
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    let descriptor = Descriptor::private()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path = wide(path)?;
    // SAFETY: private security applies before a byte can be written to a new file.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
            FILE_SHARE_READ,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful creation returned an owned file handle.
    let file = unsafe { File::from_raw_handle(handle) };
    Descriptor::from_file(&file)?.verify_private(false)?;
    Ok(file)
}

pub(super) fn open_resource(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ | READ_CONTROL)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied("bundled resource is not an ordinary private file"));
    }
    // SAFETY: the C POD output is fully initialized by a successful API call.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the file handle is live and the output buffer has the correct layout.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if information.nNumberOfLinks != 1 {
        return Err(denied("bundled resource must not be a hard link"));
    }
    Descriptor::from_file(&file)?.verify_private(false)?;
    Ok(file)
}

pub(super) fn load_path(
    path: &Path,
    _file: &File,
    _directories: &Directories,
) -> io::Result<PathBuf> {
    // Both the resource and every ancestor remain pinned without delete sharing;
    // the resource additionally excludes write sharing until load-script returns.
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{
        Authorization::SetSecurityInfo, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::WRITE_DAC;

    #[test]
    fn windows_private_cache_rejects_broad_root_and_resource_acls() {
        for change_file in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "sorotte-acl-cache-{}-{}",
                std::process::id(),
                super::super::NEXT_TEMPORARY_RESOURCE
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let path = super::super::materialize_bundled_sorotte_bridge_in(&root).unwrap();
            let target = if change_file { &path } else { &root };
            let file = OpenOptions::new()
                .access_mode(READ_CONTROL | WRITE_DAC)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(target)
                .unwrap();
            let sddl = "D:P(A;OICI;FA;;;WD)"
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let mut raw = null_mut();
            // SAFETY: fixture-only valid SDDL conversion with a live output pointer.
            assert_ne!(
                // SAFETY: the terminated fixture SDDL and output pointer remain live.
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        sddl.as_ptr(),
                        1,
                        &mut raw,
                        null_mut(),
                    )
                },
                0
            );
            let descriptor = Descriptor(raw);
            let (mut present, mut defaulted, mut acl) = (0, 0, null_mut());
            // SAFETY: the descriptor is valid and output locations remain live.
            assert_ne!(
                // SAFETY: descriptor owns a valid DACL and all outputs remain live.
                unsafe {
                    GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
                },
                0
            );
            // SAFETY: this modifies only the current test's explicitly owned fixture.
            assert_eq!(
                // SAFETY: only this test's owned file and validated DACL are modified.
                unsafe {
                    SetSecurityInfo(
                        file.as_raw_handle(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        null_mut(),
                        null_mut(),
                        acl,
                        null_mut(),
                    )
                },
                0
            );
            drop(file);
            assert_eq!(
                super::super::materialize_bundled_sorotte_bridge_in(&root)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn windows_private_descriptor_rejects_foreign_owner_even_with_restrictive_acl() {
        let sddl = "O:S-1-5-18D:P(A;OICI;FA;;;S-1-5-18)"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut raw = null_mut();
        // SAFETY: test descriptor has valid, terminated SDDL and a writable output.
        assert_ne!(
            // SAFETY: the terminated fixture SDDL and output pointer remain live.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut raw,
                    null_mut(),
                )
            },
            0
        );
        let descriptor = Descriptor(raw);
        assert_eq!(
            descriptor.verify_private(true).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
