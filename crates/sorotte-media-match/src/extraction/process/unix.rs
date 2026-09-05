use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{MediaFingerprintError, MediaToolProcessIoMetrics, tool_error};

pub(super) struct Pipe(File);

impl Pipe {
    fn new(pipe: impl IntoRawFd) -> io::Result<Self> {
        // SAFETY: IntoRawFd transfers sole ownership of this pipe descriptor.
        let file = unsafe { File::from_raw_fd(pipe.into_raw_fd()) };
        // SAFETY: file owns a live descriptor and F_GETFL has no third argument.
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags == -1
            // SAFETY: F_SETFL expects integer flags and does not retain the descriptor.
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(file))
    }

    pub(super) fn ready(&mut self) -> io::Result<bool> {
        Ok(true) // O_NONBLOCK supplies readiness and EOF without blocking.
    }
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

pub(super) struct OwnedTool {
    child: Child,
    group_id: libc::pid_t,
    finished: bool,
}

impl OwnedTool {
    pub(super) fn spawn(
        tool: &'static str,
        command: &mut Command,
        checkpoint: impl Fn() -> Result<(), MediaFingerprintError>,
    ) -> Result<(Self, Pipe, Pipe), MediaFingerprintError> {
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        checkpoint()?;
        let child = command.spawn().map_err(|error| tool_error(tool, error))?;
        let group_id = child.id() as libc::pid_t;
        let mut owned = Self {
            child,
            group_id,
            finished: false,
        };
        let setup = (|| {
            checkpoint()?;
            let stdout = owned
                .child
                .stdout
                .take()
                .ok_or_else(|| tool_error(tool, "missing stdout pipe"))?;
            let stderr = owned
                .child
                .stderr
                .take()
                .ok_or_else(|| tool_error(tool, "missing stderr pipe"))?;
            Ok((
                Pipe::new(stdout).map_err(|error| tool_error(tool, error))?,
                Pipe::new(stderr).map_err(|error| tool_error(tool, error))?,
            ))
        })();
        match setup {
            Ok((stdout, stderr)) => Ok((owned, stdout, stderr)),
            Err(error) => {
                if let Err(cleanup) = owned.finish(super::CLEANUP_TIMEOUT) {
                    return Err(tool_error(
                        tool,
                        format!("{error}; owned-process cleanup incomplete: {cleanup}"),
                    ));
                }
                Err(error)
            }
        }
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(super) fn io_counters(&self) -> MediaToolProcessIoMetrics {
        MediaToolProcessIoMetrics::default()
    }

    fn terminate_group(&self) -> io::Result<()> {
        // SAFETY: a negative ID targets only the process group created for this
        // owned child. No other caller can join it through our API.
        if unsafe { libc::kill(-self.group_id, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, timeout: Duration) -> io::Result<()> {
        self.terminate_group()?;
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() && !self.group_has_live_members()? {
                self.finished = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "media process was not reaped after SIGKILL",
                ));
            }
            thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn group_has_live_members(&self) -> io::Result<bool> {
        // SAFETY: signal zero queries only the group created for this child.
        if unsafe { libc::kill(-self.group_id, 0) } == -1 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(false)
            } else {
                Err(error)
            };
        }
        #[cfg(target_os = "linux")]
        {
            // Reparented descendants may remain as init-owned zombies. They are
            // terminated and hold no pipes, even on hosts whose PID 1 delays
            // reaping. Confirm there are no executing members of our group.
            for entry in std::fs::read_dir("/proc")? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                    continue;
                }
                let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                    Ok(stat) => stat,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                let Some((_, fields)) = stat.rsplit_once(") ") else {
                    continue;
                };
                let mut fields = fields.split_whitespace();
                let state = fields.next();
                let group = fields
                    .nth(1)
                    .and_then(|value| value.parse::<libc::pid_t>().ok());
                if group == Some(self.group_id) && !matches!(state, Some("Z" | "X")) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(true)
        }
    }
}

impl Drop for OwnedTool {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.terminate_group();
            let _ = self.child.try_wait();
        }
    }
}
