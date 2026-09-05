//! Security is supplied to CreateFileW, before the file exists or contains bytes.
use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::null_mut,
};

use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_ALWAYS, READ_CONTROL,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: Both descriptor constructors below return LocalAlloc-owned memory.
        unsafe {
            LocalFree(self.0);
        }
    }
}

impl SecurityDescriptor {
    pub(super) fn for_destination(destination: &Path) -> io::Result<Self> {
        let existing = OpenOptions::new()
            .access_mode(READ_CONTROL)
            .open(destination);
        match existing {
            Ok(file) => {
                let descriptor = Self::from_file(&file)?;
                // Explicit protected descriptors are a user's security policy. Keep
                // their owner and DACL byte-for-byte, including stricter deny rules.
                // Inherited and NULL DACLs are replaced with a private descriptor.
                if descriptor.has_protected_dacl()? {
                    return Ok(descriptor);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Self::owner_only()
    }

    pub(super) fn from_file(file: &File) -> io::Result<Self> {
        let mut descriptor = null_mut();
        // SAFETY: The live file handle is valid; the output receives an allocated
        // self-relative descriptor, freed by SecurityDescriptor::drop.
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

    fn has_protected_dacl(&self) -> io::Result<bool> {
        let mut control = 0;
        let mut revision = 0;
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        // SAFETY: self owns a valid descriptor and all out pointers remain live.
        let valid = unsafe {
            GetSecurityDescriptorControl(self.0, &mut control, &mut revision) != 0
                && GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) != 0
        };
        if !valid {
            return Err(io::Error::last_os_error());
        }
        Ok(control & SE_DACL_PROTECTED != 0 && present != 0 && !dacl.is_null())
    }

    pub(super) fn owner_only() -> io::Result<Self> {
        Self::owner_only_with_inheritance(false)
    }

    fn owner_only_with_inheritance(inherit: bool) -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and token is an out pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken returned an owned handle above.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut length = 0;
        // SAFETY: The first call queries the necessary length without a buffer.
        unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut length);
        }
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        // usize storage provides alignment for TOKEN_USER and its embedded SID.
        let mut storage = vec![0_usize; (length as usize).div_ceil(size_of::<usize>())];
        // SAFETY: storage has sufficient size and alignment and the handle is live.
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
        // SAFETY: A successful TokenUser query initialized TOKEN_USER at this address.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let mut sid_string = null_mut();
        // SAFETY: The SID is inside live storage; sid_string receives LocalAlloc memory.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_string) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: The API produced a NUL-terminated UTF-16 SID string.
        let sid = unsafe {
            let mut count = 0;
            while *sid_string.add(count) != 0 {
                count += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_string, count));
            LocalFree(sid_string.cast());
            sid
        };
        let inheritance = if inherit { "OICI" } else { "" };
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;{inheritance};FA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = null_mut();
        // SAFETY: sddl is NUL terminated; the allocated result is owned by Self.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    pub(super) fn create_file(&self, path: &Path, exclusive: bool) -> io::Result<File> {
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if path_wide[..path_wide.len() - 1].contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "settings path contains NUL",
            ));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        };
        // SAFETY: Path and descriptor stay alive during creation. No file is ever
        // created with inherited permissions, even while still empty. Excluding
        // FILE_SHARE_DELETE also prevents unlinking the persistent lock inode.
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &attributes,
                if exclusive { CREATE_NEW } else { OPEN_ALWAYS },
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW returned a new owned file handle.
        let file = unsafe { File::from_raw_handle(handle) };
        if exclusive {
            let verified =
                Self::from_file(&file).and_then(|descriptor| descriptor.has_protected_dacl());
            match verified {
                Ok(true) => {}
                result => {
                    drop(file);
                    let _ = std::fs::remove_file(path);
                    return Err(result.err().unwrap_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "filesystem did not apply the protected settings DACL",
                        )
                    }));
                }
            }
        }
        Ok(file)
    }

    pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            CreateDirectoryW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let descriptor = Self::owner_only_with_inheritance(true)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private directory path contains NUL",
            ));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: All buffers and the descriptor stay live. CreateDirectoryW
        // fails for any existing entry, including a junction or reparse point.
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let verified = OpenOptions::new()
            .access_mode(READ_CONTROL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .and_then(|file| Self::from_file(&file))
            .and_then(|descriptor| descriptor.has_protected_dacl());
        match verified {
            Ok(true) => Ok(()),
            result => {
                let _ = std::fs::remove_dir(path);
                Err(result.err().unwrap_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "filesystem did not apply the protected directory DACL",
                    )
                }))
            }
        }
    }
}
