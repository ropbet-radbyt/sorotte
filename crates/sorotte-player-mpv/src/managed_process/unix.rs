use super::*;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

#[derive(Debug)]
pub(super) struct PlatformProcess {
    child: Mutex<Child>,
    id: u32,
    group: bool,
}

impl PlatformProcess {
    pub(super) fn spawn(spec: &ManagedMpvCommand) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let child = launch_from_process_lifetime_thread(spec)?;
        #[cfg(not(target_os = "linux"))]
        let child = spawn_child(spec)?;
        Ok(Self {
            id: child.id(),
            child: Mutex::new(child),
            group: true,
        })
    }

    #[cfg(feature = "test-support")]
    pub(super) fn adopt(child: Child) -> io::Result<Self> {
        Ok(Self {
            id: child.id(),
            child: Mutex::new(child),
            group: false,
        })
    }

    pub(super) fn id(&self) -> u32 {
        self.id
    }

    pub(super) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        #[cfg(target_os = "linux")]
        if self.group {
            use std::os::unix::process::ExitStatusExt;
            // Keep the exited leader waitable until group cleanup. Reaping it
            // during a health poll would permit numeric PID/process-group reuse.
            // SAFETY: siginfo_t is C POD and waitid initializes the output.
            let mut information: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: this is our retained child; WNOWAIT only observes its state.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.id,
                    &mut information,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == 0 {
                // SAFETY: successful waitid initialized the SIGCHLD fields.
                let pid = unsafe { information.si_pid() };
                if pid == 0 {
                    return Ok(None);
                }
                // SAFETY: a nonzero child PID identifies initialized status fields.
                let status = unsafe { information.si_status() };
                return Ok(Some(ExitStatus::from_raw(
                    if information.si_code == libc::CLD_EXITED {
                        status << 8
                    } else {
                        status
                    },
                )));
            }
            if io::Error::last_os_error().raw_os_error() != Some(libc::ECHILD) {
                return Err(io::Error::last_os_error());
            }
            // Already completed cleanup left std::process::Child's cached status.
        }
        self.child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_wait()
    }

    pub(super) fn terminate_until(&self, deadline: Instant) -> io::Result<()> {
        // A test-adopted child may already have been reaped. Its numeric PID is
        // no longer safe to signal; production groups retain their own identity.
        if !self.group && self.try_wait()?.is_some() {
            return Ok(());
        }
        let id = if self.group {
            -(self.id as libc::pid_t)
        } else {
            self.id as libc::pid_t
        };
        // SAFETY: only the explicitly owned child or its private group is targeted.
        if unsafe { libc::kill(id, libc::SIGKILL) } == -1
            && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        {
            return Err(io::Error::last_os_error());
        }
        loop {
            if self.try_wait()?.is_some() && (!self.group || !self.group_has_live_members()?) {
                self.child
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .try_wait()?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned mpv did not exit before shutdown deadline",
                ));
            }
            std::thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn group_has_live_members(&self) -> io::Result<bool> {
        // SAFETY: signal zero queries only the process group created for this child.
        if unsafe { libc::kill(-(self.id as libc::pid_t), 0) } == -1 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(false)
            } else {
                Err(error)
            };
        }
        #[cfg(target_os = "linux")]
        {
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
                let group = fields.nth(1).and_then(|value| value.parse::<u32>().ok());
                if group == Some(self.id) && !matches!(state, Some("Z" | "X")) {
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

fn spawn_child(spec: &ManagedMpvCommand) -> io::Result<Child> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.environment.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(directory) = &spec.current_dir {
        command.current_dir(directory);
    }
    #[cfg(target_os = "linux")]
    {
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: the pre-exec closure uses only async-signal-safe libc calls
        // and does not allocate, lock, or inspect shared Rust state.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(io::Error::last_os_error());
                }
                // The parent may have exited between fork and prctl.
                if libc::getppid() != parent {
                    libc::_exit(127);
                }
                Ok(())
            });
        }
    }
    command.spawn()
}

#[cfg(target_os = "linux")]
fn launch_from_process_lifetime_thread(spec: &ManagedMpvCommand) -> io::Result<Child> {
    use std::sync::{OnceLock, mpsc};
    type Launch = (ManagedMpvCommand, mpsc::SyncSender<io::Result<Child>>);
    // Linux PDEATHSIG follows the creating thread. One process-lifetime launch
    // service preserves parent-exit containment when guards move between runtime
    // threads. It never owns cleanup; returned children belong to their guards.
    static LAUNCHER: OnceLock<Result<mpsc::SyncSender<Launch>, String>> = OnceLock::new();
    let launcher = LAUNCHER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Launch>(8);
        std::thread::Builder::new()
            .name("sorotte-owned-player-launch".into())
            .spawn(move || {
                while let Ok((command, completed)) = rx.recv() {
                    let result = spawn_child(&command);
                    if let Err(mpsc::SendError(Ok(mut child))) = completed.send(result) {
                        // The synchronous caller vanished unexpectedly. Deliver the
                        // kill here; no background cleanup is advertised as complete.
                        let _ = child.kill();
                        let deadline = Instant::now() + DROP_TIMEOUT;
                        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(tx)
    });
    let launcher = launcher
        .as_ref()
        .map_err(|error| io::Error::other(error.clone()))?;
    let (tx, rx) = mpsc::sync_channel(1);
    launcher
        .send((spec.clone(), tx))
        .map_err(io::Error::other)?;
    rx.recv().map_err(io::Error::other)?
}
