use super::*;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU64;

const FIXTURE: &str = "managed_process::tests::process_fixture";
const ROLE: &str = "SOROTTE_OWNED_MPV_FIXTURE_ROLE";
const ROOT: &str = "SOROTTE_OWNED_MPV_FIXTURE_ROOT";
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sorotte-owned-mpv-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn marker(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn command(&self, role: &str) -> ManagedMpvCommand {
        let mut command = ManagedMpvCommand::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env(ROLE, role)
            .env(ROOT, &self.0);
        command
    }
    fn external(&self, role: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env(ROLE, role)
            .env(ROOT, &self.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }
    fn wait(&self, name: &str) {
        wait_until(Duration::from_secs(10), || self.marker(name).exists());
    }
    fn endpoint(&self) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!(
                r"\\.\pipe\{}",
                self.0.file_name().unwrap().to_string_lossy()
            ))
        }
        #[cfg(unix)]
        {
            self.marker("mpv.sock")
        }
    }
    fn cleanup_path(&self) -> Option<PathBuf> {
        #[cfg(windows)]
        {
            None
        }
        #[cfg(unix)]
        {
            Some(self.endpoint())
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "process fixture exceeded {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
fn endpoint(path: &Path) -> std::os::windows::io::OwnedHandle {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
        System::Pipes::CreateNamedPipeW,
    };
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: terminated name and fixed bounds create one owned fixture pipe.
    let handle = unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            0,
            1,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(
        handle,
        INVALID_HANDLE_VALUE,
        "endpoint must be reusable: {}",
        io::Error::last_os_error()
    );
    // SAFETY: the successful call returned a new owned handle.
    unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) }
}
#[cfg(unix)]
fn endpoint(path: &Path) -> std::os::unix::net::UnixListener {
    std::os::unix::net::UnixListener::bind(path).unwrap()
}

#[cfg(windows)]
struct Observer(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl Observer {
    fn new(pid: u32) -> Self {
        use std::os::windows::io::FromRawHandle;
        // SAFETY: synchronization access retains the precise live fixture process.
        let handle = unsafe {
            windows_sys::Win32::System::Threading::OpenProcess(
                windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        assert!(!handle.is_null());
        // SAFETY: OpenProcess returned a new owned handle.
        Self(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) })
    }
    fn alive(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: synchronization handle remains owned throughout observation.
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(self.0.as_raw_handle(), 0)
                == windows_sys::Win32::Foundation::WAIT_TIMEOUT
        }
    }
}
#[cfg(unix)]
struct Observer(u32);
#[cfg(unix)]
impl Observer {
    fn new(pid: u32) -> Self {
        Self(pid)
    }
    fn alive(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string(format!("/proc/{}/stat", self.0))
                .ok()
                .and_then(|stat| {
                    stat.rsplit_once(") ")
                        .map(|(_, rest)| !rest.starts_with(['Z', 'X']))
                })
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            unsafe { libc::kill(self.0 as libc::pid_t, 0) == 0 }
        }
    }
}

#[test]
#[ignore = "subprocess entry point, invoked by process contract tests"]
fn process_fixture() {
    let role = std::env::var(ROLE).unwrap();
    let fixture = Fixture(PathBuf::from(std::env::var_os(ROOT).unwrap()));
    if role == "owned" || role == "external" {
        let _endpoint = (role == "owned").then(|| endpoint(&fixture.endpoint()));
        std::fs::write(fixture.marker(&role), std::process::id().to_string()).unwrap();
        // A watchdog bounds failed fixtures without allowing voluntary exit to
        // satisfy the much shorter parent-observation deadline.
        std::thread::sleep(Duration::from_secs(20));
        std::process::exit(9);
    }
    let scope = ManagedMpvShutdownScope::default();
    let child = fixture
        .command("owned")
        .spawn(fixture.cleanup_path())
        .unwrap();
    scope.register(&child).unwrap();
    fixture.wait("owned");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _child = child;
        entered_tx.send(()).unwrap();
        loop {
            std::thread::park();
        }
    });
    entered_rx.recv().unwrap();
    std::fs::write(
        fixture.marker("blocked"),
        b"worker owns child and never releases",
    )
    .unwrap();
    fixture.wait("exit-parent");
    if role == "bounded-parent" {
        scope
            .terminate_until(Instant::now() + Duration::from_millis(500))
            .unwrap();
        std::fs::write(fixture.marker("terminated"), b"observed").unwrap();
    }
    // Deliberately bypass Rust destructors with the owner permanently blocked.
    std::process::exit(0);
}

fn blocked_parent_exit(role: &str) {
    let fixture = Fixture::new();
    let mut external = fixture.external("external");
    fixture.wait("external");
    let mut parent = fixture.external(role);
    fixture.wait("blocked");
    let pid = std::fs::read_to_string(fixture.marker("owned"))
        .unwrap()
        .parse()
        .unwrap();
    let owned = Observer::new(pid);
    assert!(owned.alive());
    std::fs::write(fixture.marker("exit-parent"), b"exit").unwrap();
    wait_until(Duration::from_secs(2), || {
        parent.try_wait().unwrap().is_some()
    });
    wait_until(Duration::from_millis(750), || !owned.alive());
    assert!(
        external.try_wait().unwrap().is_none(),
        "external attachment must survive owner exit"
    );
    if role == "bounded-parent" {
        assert!(fixture.marker("terminated").exists());
        let _reused = endpoint(&fixture.endpoint());
    }
    external.kill().unwrap();
    external.wait().unwrap();
}

#[test]
fn independent_scope_terminates_child_when_blocked_owner_parent_exits() {
    blocked_parent_exit("bounded-parent");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn platform_containment_terminates_child_on_abrupt_parent_exit() {
    blocked_parent_exit("abrupt-parent");
}

#[test]
fn owned_shutdown_replacement_and_late_drop_preserve_reused_endpoint() {
    let fixture = Fixture::new();
    let scope = ManagedMpvShutdownScope::default();
    let _entered = scope.enter();
    let child = fixture
        .command("owned")
        .spawn(fixture.cleanup_path())
        .unwrap();
    fixture.wait("owned");
    let observed = Observer::new(child.id());
    scope
        .terminate_until(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(!observed.alive());
    let _reused = endpoint(&fixture.endpoint());
    drop(child);
    #[cfg(unix)]
    assert!(
        fixture.endpoint().exists(),
        "late guard drop must not unlink replacement IPC"
    );
    assert_eq!(
        fixture.command("owned").spawn(None).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn owned_launch_failure_leaves_scope_usable() {
    let fixture = Fixture::new();
    let scope = ManagedMpvShutdownScope::default();
    let _entered = scope.enter();
    assert!(
        ManagedMpvCommand::new(fixture.marker("missing"))
            .spawn(None)
            .is_err()
    );
    let child = fixture
        .command("owned")
        .spawn(fixture.cleanup_path())
        .unwrap();
    fixture.wait("owned");
    let observed = Observer::new(child.id());
    drop(child);
    assert!(!observed.alive());
}

#[test]
fn owned_child_survives_transfer_from_a_short_lived_launcher_thread() {
    let fixture = Fixture::new();
    let command = fixture.command("owned");
    let cleanup = fixture.cleanup_path();
    let child = std::thread::spawn(move || command.spawn(cleanup).unwrap())
        .join()
        .unwrap();
    fixture.wait("owned");
    let observer = Observer::new(child.id());
    assert!(
        observer.alive(),
        "launching thread exit must not impersonate parent process exit"
    );
    drop(child);
    assert!(!observer.alive());
}
