use std::fs::File;
use std::io::{self, Read};
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessIoCounters, IO_COUNTERS, OpenProcess, PROCESS_SYNCHRONIZE,
    ResumeThread, WaitForSingleObject,
};

use super::{MediaFingerprintError, MediaToolProcessIoMetrics, tool_error};

mod launch;

pub(super) struct Pipe {
    file: File,
    eof: bool,
}

impl Pipe {
    fn new(pipe: impl IntoRawHandle) -> Self {
        // SAFETY: IntoRawHandle transfers sole ownership of this pipe handle.
        let file = unsafe { File::from_raw_handle(pipe.into_raw_handle()) };
        Self { file, eof: false }
    }

    pub(super) fn ready(&mut self) -> io::Result<bool> {
        let mut available = 0;
        // SAFETY: the file owns a live pipe handle and `available` is writable.
        let ok = unsafe {
            PeekNamedPipe(
                self.file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            return Ok(available != 0);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            self.eof = true;
            Ok(true)
        } else {
            Err(error)
        }
    }
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.eof {
            Ok(0)
        } else {
            self.file.read(buffer)
        }
    }
}

pub(super) struct OwnedTool {
    process: OwnedHandle,
    job: OwnedHandle,
    finished: bool,
}

impl OwnedTool {
    pub(super) fn spawn(
        tool: &'static str,
        command: &mut Command,
        checkpoint: impl Fn() -> Result<(), MediaFingerprintError>,
    ) -> Result<(Self, Pipe, Pipe), MediaFingerprintError> {
        Self::spawn_observed(tool, command, checkpoint, |_| {})
    }

    pub(super) fn spawn_observed(
        tool: &'static str,
        command: &Command,
        checkpoint: impl Fn() -> Result<(), MediaFingerprintError>,
        created: impl FnOnce(u32),
    ) -> Result<(Self, Pipe, Pipe), MediaFingerprintError> {
        let job = create_job().map_err(|error| tool_error(tool, error))?;
        let launch::Launched {
            process,
            thread,
            id,
            stdout,
            stderr,
        } = launch::spawn(command, &job, || checkpoint().map_err(io::Error::other)).map_err(
            |error| {
                checkpoint()
                    .err()
                    .unwrap_or_else(|| tool_error(tool, error))
            },
        )?;
        // The kernel has attached the private job before returning the suspended
        // process. Even abrupt owner exit here closes the job and kills the child.
        created(id);
        let mut owned = Self {
            process,
            job,
            finished: false,
        };
        let setup = (|| {
            checkpoint()?;
            // SAFETY: CreateProcessW returned the suspended primary thread;
            // process creation already placed it in the private kill-on-close job.
            if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                return Err(tool_error(tool, io::Error::last_os_error()));
            }
            Ok(())
        })();
        if let Err(error) = setup {
            if let Err(cleanup) = owned.finish(super::CLEANUP_TIMEOUT) {
                return Err(tool_error(
                    tool,
                    format!("{error}; owned-process cleanup incomplete: {cleanup}"),
                ));
            }
            return Err(error);
        }
        Ok((owned, stdout, stderr))
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: the owned process handle has SYNCHRONIZE access and is live.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: the process has exited and the output buffer is writable.
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(ExitStatus::from_raw(code)))
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn io_counters(&self) -> MediaToolProcessIoMetrics {
        // SAFETY: IO_COUNTERS is a C POD structure whose all-zero state is valid.
        let mut counters: IO_COUNTERS = unsafe { zeroed() };
        // SAFETY: our process handle is live, and counters is a writable IO_COUNTERS.
        if unsafe { GetProcessIoCounters(self.process.as_raw_handle(), &mut counters) } == 0 {
            return MediaToolProcessIoMetrics::default();
        }
        MediaToolProcessIoMetrics {
            read_bytes: Some(counters.ReadTransferCount),
            read_ops: Some(counters.ReadOperationCount),
        }
    }

    pub(super) fn finish(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        // Job accounting reaches zero before a terminated descendant's process
        // handle necessarily signals. Retain exact handles before termination,
        // then observe those exit signals as well as the empty job.
        let before = self.accounting();
        let process_handles = self.process_handles();
        // SAFETY: our job handle is live, private, and configured without breakaway.
        let job_result = unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) };
        let job_error = (job_result == 0).then(io::Error::last_os_error);
        let before = before?;
        let process_handles = process_handles?;
        loop {
            let reaped = self.try_wait()?.is_some();
            let accounting = self.accounting()?;
            let mut all_exited = true;
            for process in &process_handles {
                // SAFETY: every handle is retained from this private job and
                // SYNCHRONIZE access permits a nonblocking process-exit query.
                match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
                    WAIT_OBJECT_0 => {}
                    WAIT_TIMEOUT => all_exited = false,
                    _ => return Err(io::Error::last_os_error()),
                }
            }
            if reaped && accounting.ActiveProcesses == 0 && all_exited {
                if accounting.TotalProcesses != before.TotalProcesses {
                    return Err(io::Error::other(
                        "job gained a process during cleanup; all exit signals could not be verified",
                    ));
                }
                self.finished = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(job_error.unwrap_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "media process job still has live processes",
                    )
                }));
            }
            thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn accounting(&self) -> io::Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
        // SAFETY: this C POD structure permits an all-zero initial state.
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        // SAFETY: the live job and writable buffer have the requested type and size.
        if unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(accounting)
        }
    }

    fn process_handles(&self) -> io::Result<Vec<OwnedHandle>> {
        // ffmpeg/ffprobe normally launch no children. Bound even the cleanup
        // snapshot; exceeding this capacity still terminates the entire job but
        // reports that complete exit observation was unavailable.
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
        // SAFETY: ProcessList extends JOBOBJECT_BASIC_PROCESS_ID_LIST with a
        // correctly aligned bounded trailing PID array of the supplied size.
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
            return Err(io::Error::other("job process observation limit exceeded"));
        }
        let mut handles = Vec::new();
        for &pid in &list.ids[..list.count as usize] {
            // SAFETY: the snapshot identifies a process in our private job;
            // requesting only SYNCHRONIZE cannot change that process.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid as u32) };
            if handle.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                    continue;
                }
                return Err(error);
            }
            // SAFETY: OpenProcess returned a new owned synchronization handle.
            handles.push(unsafe { OwnedHandle::from_raw_handle(handle) });
        }
        Ok(handles)
    }
}

impl Drop for OwnedTool {
    fn drop(&mut self) {
        if !self.finished {
            // SAFETY: the job remains live until after this destructor returns.
            unsafe {
                TerminateJobObject(self.job.as_raw_handle(), 1);
            }
            let _ = self.try_wait();
        }
        // Closing the job is an additional kernel-owned kill-on-close guarantee.
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null security/name parameters create an unnamed, noninheritable job.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateJobObjectW returned a new owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: this C POD structure permits an all-zero initial state.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the buffer has the exact requested layout, size, and live lifetime.
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
