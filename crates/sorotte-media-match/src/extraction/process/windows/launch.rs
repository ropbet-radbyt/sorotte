//! Atomically contain helpers and inherit only their three standard handles.

use super::Pipe;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    process::Command,
};
use windows_sys::Win32::{
    Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        Pipes::CreatePipe,
        Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, CREATE_SUSPENDED,
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
        },
    },
};

pub(super) struct Launched {
    pub process: OwnedHandle,
    pub thread: OwnedHandle,
    pub id: u32,
    pub stdout: Pipe,
    pub stderr: Pipe,
}

pub(super) fn spawn(
    command: &Command,
    job: &OwnedHandle,
    checkpoint: impl FnOnce() -> io::Result<()>,
) -> io::Result<Launched> {
    let null = null_stdio()?;
    let (stdout, stdout_writer) = output_pipe()?;
    let (stderr, stderr_writer) = output_pipe()?;
    let job_handles = [job.as_raw_handle()];
    let stdio_handles = [
        null.as_raw_handle(),
        stdout_writer.as_raw_handle(),
        stderr_writer.as_raw_handle(),
    ];
    let mut attributes = AttributeList::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &job_handles)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &stdio_handles)?;
    let mut command_line = quoted_argument(command.get_program())?;
    for arg in command.get_args() {
        command_line.push(b' ' as u16);
        command_line.extend(quoted_argument(arg)?);
    }
    command_line.push(0);
    let mut environment = environment_block(command)?;
    let directory = command
        .get_current_dir()
        .map(|path| nul_terminated(path.as_os_str()))
        .transpose()?;
    // SAFETY: Win32 startup/process structures permit all-zero initialization;
    // all required size, handle, and attribute fields are populated before use.
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = null.as_raw_handle();
    startup.StartupInfo.hStdOutput = stdout_writer.as_raw_handle();
    startup.StartupInfo.hStdError = stderr_writer.as_raw_handle();
    startup.lpAttributeList = attributes.pointer();
    // SAFETY: PROCESS_INFORMATION is a C POD output buffer.
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    checkpoint()?;
    // SAFETY: all buffers are terminated and live, the explicit inheritance list
    // contains only the standard handles, and the job is attached atomically.
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | BELOW_NORMAL_PRIORITY_CLASS
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_mut_ptr().cast(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateProcessW returned two distinct, owned handles.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    // SAFETY: the primary thread handle is a separate owned handle.
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    Ok(Launched {
        process,
        thread,
        id: info.dwProcessId,
        stdout,
        stderr,
    })
}

fn output_pipe() -> io::Result<(Pipe, OwnedHandle)> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut reader = std::ptr::null_mut();
    let mut writer = std::ptr::null_mut();
    // SAFETY: the output pointers are distinct and security has the required size.
    if unsafe { CreatePipe(&mut reader, &mut writer, &security, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreatePipe returned distinct owned handles.
    let reader = unsafe { OwnedHandle::from_raw_handle(reader) };
    // SAFETY: the pipe's writer is independently owned.
    let writer = unsafe { OwnedHandle::from_raw_handle(writer) };
    // SAFETY: only this new reader is affected. The child inherits its writer.
    if unsafe { SetHandleInformation(reader.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((Pipe::new(reader), writer))
}

fn environment_block(command: &Command) -> io::Result<Vec<u16>> {
    // Production tool commands inherit the environment. Fixture commands add or
    // remove explicit values; this internal launcher does not accept env_clear.
    let mut values = BTreeMap::new();
    for (name, value) in std::env::vars_os() {
        values.insert(name.to_string_lossy().to_uppercase(), (name, value));
    }
    for (name, value) in command.get_envs() {
        let key = name.to_string_lossy().to_uppercase();
        if let Some(value) = value {
            values.insert(key, (name.to_os_string(), value.to_os_string()));
        } else {
            values.remove(&key);
        }
    }
    let mut block = Vec::new();
    for (_, (name, value)) in values {
        let mut entry = name;
        entry.push("=");
        entry.push(value);
        block.extend(nul_terminated(&entry)?);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

fn null_stdio() -> io::Result<OwnedHandle> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: the NUL path is terminated; the explicit handle-inheritance list
    // prevents any other inheritable application handles reaching the player.
    let handle = unsafe {
        CreateFileW(
            [b'N' as u16, b'U' as u16, b'L' as u16, 0].as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &security,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a new owned file handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

struct AttributeList {
    memory: Vec<usize>,
    initialized: bool,
}
impl AttributeList {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: a null buffer queries the required allocation size.
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut result = Self {
            memory: vec![0; bytes.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        // SAFETY: the allocation is pointer-aligned and at least the queried size.
        if unsafe { InitializeProcThreadAttributeList(result.pointer(), 2, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        result.initialized = true;
        Ok(result)
    }
    fn pointer(&mut self) -> *mut std::ffi::c_void {
        self.memory.as_mut_ptr().cast()
    }
    fn set(&mut self, kind: usize, handles: &[HANDLE]) -> io::Result<()> {
        // SAFETY: the handle arrays remain live through CreateProcessW and the
        // attribute list allocation remains stable for this object's lifetime.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                kind,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: the initialized list remains allocated until after deletion.
            unsafe {
                DeleteProcThreadAttributeList(self.pointer());
            }
        }
    }
}

fn nul_terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide = value.encode_wide().collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process launch value contains NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn quoted_argument(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide = nul_terminated(value)?;
    wide.pop();
    let mut quoted = vec![b'"' as u16];
    let mut slashes = 0;
    for character in wide {
        if character == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            b'\\' as u16,
            if character == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        quoted.push(character);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    quoted.push(b'"' as u16);
    Ok(quoted)
}
