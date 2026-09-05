use super::*;
use std::{
    collections::BTreeMap,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_INVALID_PARAMETER, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, OpenProcess, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION, PROCESS_SYNCHRONIZE,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

#[derive(Debug)]
pub(super) struct PlatformProcess {
    process: OwnedHandle,
    job: OwnedHandle,
    id: u32,
}

impl PlatformProcess {
    pub(super) fn spawn(spec: &ManagedMpvCommand) -> io::Result<Self> {
        let job = create_job()?;
        let null = null_stdio()?;
        let mut attributes = AttributeList::new()?;
        let job_handles = [job.as_raw_handle()];
        let stdio_handles = [null.as_raw_handle()];
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &job_handles)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &stdio_handles)?;
        let mut command_line = quoted_argument(spec.program.as_os_str())?;
        for arg in &spec.args {
            command_line.push(b' ' as u16);
            command_line.extend(quoted_argument(arg)?);
        }
        command_line.push(0);
        let mut environment = environment_block(&spec.environment)?;
        let directory = spec
            .current_dir
            .as_ref()
            .map(|path| nul_terminated(path.as_os_str()))
            .transpose()?;
        // SAFETY: Win32 startup/process structures permit all-zero initialization;
        // required size, handle, and attribute fields are populated before use.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = null.as_raw_handle();
        startup.StartupInfo.hStdOutput = null.as_raw_handle();
        startup.StartupInfo.hStdError = null.as_raw_handle();
        startup.lpAttributeList = attributes.pointer();
        // SAFETY: PROCESS_INFORMATION is a C POD output buffer.
        let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
        // SAFETY: all UTF-16 buffers are terminated and live, only the explicit
        // NUL handle is inherited, and the job list attaches containment atomically
        // as the kernel creates the process, before any child instruction runs.
        if unsafe {
            CreateProcessW(
                std::ptr::null(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_mut_ptr().cast(),
                directory
                    .as_ref()
                    .map_or(std::ptr::null(), |path| path.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateProcessW returned two new owned handles.
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        // SAFETY: the initial thread handle is a separate owned handle.
        let _thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
        Ok(Self {
            process,
            job,
            id: info.dwProcessId,
        })
    }

    #[cfg(feature = "test-support")]
    pub(super) fn adopt(mut child: std::process::Child) -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let id = child.id();
        let job = create_job()?;
        // SAFETY: the child is explicitly transferred by a lifecycle fixture.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
            let error = io::Error::last_os_error();
            // An already exited fixture needs no containment.
            if child.try_wait()?.is_none() {
                let _ = child.kill();
                return Err(error);
            }
        }
        let process: OwnedHandle = child.into();
        Ok(Self { process, job, id })
    }

    pub(super) fn id(&self) -> u32 {
        self.id
    }

    pub(super) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: the owned process handle has SYNCHRONIZE rights and is live.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                // SAFETY: the process has signaled exit; exit_code is writable.
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut exit_code) } == 0
                {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(ExitStatus::from_raw(exit_code)))
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        let before = self.accounting();
        let descendants = self.process_handles();
        // SAFETY: this private kill-on-close job contains only the owned player tree.
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let before = before?;
        let descendants = descendants?;
        loop {
            let mut all_exited = self.try_wait()?.is_some();
            for child in &descendants {
                // SAFETY: snapshot handles retain their exact process identity.
                match unsafe { WaitForSingleObject(child.as_raw_handle(), 0) } {
                    WAIT_OBJECT_0 => {}
                    WAIT_TIMEOUT => all_exited = false,
                    _ => return Err(io::Error::last_os_error()),
                }
            }
            let accounting = self.accounting()?;
            if all_exited && accounting.ActiveProcesses == 0 {
                if accounting.TotalProcesses != before.TotalProcesses {
                    return Err(io::Error::other(
                        "owned player tree changed during cleanup; complete exit observation was unavailable",
                    ));
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned player did not exit before shutdown deadline",
                ));
            }
            std::thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn accounting(&self) -> io::Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
        // SAFETY: this C POD structure permits an all-zero initial state.
        let mut value: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        // SAFETY: the live job and writable output buffer have the requested layout.
        if unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut value as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(value)
        }
    }

    fn process_handles(&self) -> io::Result<Vec<OwnedHandle>> {
        #[repr(C)]
        struct ProcessList {
            assigned: u32,
            count: u32,
            ids: [usize; 256],
        }
        let mut list = ProcessList {
            assigned: 0,
            count: 0,
            ids: [0; 256],
        };
        // SAFETY: this bounded trailing PID array extends the documented C layout.
        if unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                (&mut list as *mut ProcessList).cast(),
                size_of::<ProcessList>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if list.count as usize > list.ids.len() {
            return Err(io::Error::other("owned process observation limit exceeded"));
        }
        let mut handles = Vec::new();
        for &id in &list.ids[..list.count as usize] {
            // SAFETY: only synchronization access to a private job member is requested.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id as u32) };
            if handle.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                    continue;
                }
                return Err(error);
            }
            // SAFETY: OpenProcess returned a new owned process handle.
            handles.push(unsafe { OwnedHandle::from_raw_handle(handle) });
        }
        Ok(handles)
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null parameters create an unnamed noninheritable job.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateJobObjectW returned a new owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: this C POD structure permits an all-zero initial state.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the buffer is the requested information class and exact size.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(job)
    }
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

fn environment_block(overrides: &[(OsString, OsString)]) -> io::Result<Vec<u16>> {
    let mut values = BTreeMap::new();
    for (name, value) in std::env::vars_os().chain(overrides.iter().cloned()) {
        let key = name.to_string_lossy().to_uppercase();
        values.insert(key, (name, value));
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
