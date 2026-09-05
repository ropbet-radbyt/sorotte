//! The parent really exits while its runtime worker permanently owns the player.
use super::*;
use sorotte_player_mpv::managed_process::{
    ManagedMpvCommand, ManagedMpvShutdownScope, OwnedMpvProcess,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::AtomicU64,
};

const ENTRY: &str = "app::runtime_queue::tests::process_shutdown::fixture_entrypoint";
const ROLE: &str = "SOROTTE_GUI_BLOCKED_OWNER_FIXTURE_ROLE";
const ROOT: &str = "SOROTTE_GUI_BLOCKED_OWNER_FIXTURE_ROOT";
static NEXT: AtomicU64 = AtomicU64::new(1);

fn wait_marker(root: &Path, marker: &str) {
    wait_until(Duration::from_secs(10), marker, || {
        root.join(marker).exists()
    });
}
fn external(root: &Path, role: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", ENTRY, "--ignored", "--nocapture"])
        .env(ROLE, role)
        .env(ROOT, root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}
fn ipc_path(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\{}",
            root.file_name().unwrap().to_string_lossy()
        ))
    }
    #[cfg(unix)]
    {
        root.join("owned.sock")
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
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: terminated path and fixed bounds create a single fixture endpoint.
    let raw = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
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
        raw,
        INVALID_HANDLE_VALUE,
        "same IPC name must be reusable: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: successful creation returned a new owned handle.
    unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) }
}
#[cfg(unix)]
fn endpoint(path: &Path) -> std::os::unix::net::UnixListener {
    std::os::unix::net::UnixListener::bind(path).unwrap()
}

struct BlockedOwner {
    child: OwnedMpvProcess,
    root: PathBuf,
}
impl GuiQueuedRuntimeOwner for BlockedOwner {
    fn register_owned_processes(&self, scope: &ManagedMpvShutdownScope) -> Result<(), String> {
        scope
            .register(&self.child)
            .map_err(|error| error.to_string())
    }
    fn input_changed(
        &mut self,
        _: &GuiQueuedRuntimeBridgeHandle,
        _: &crate::app::feature_slices::GuiRuntimeInput,
    ) {
    }
    fn poll(&mut self, _: &GuiQueuedRuntimeBridgeHandle) {
        fs::write(self.root.join("owner-blocked"), b"blocked forever").unwrap();
        loop {
            std::thread::park();
        }
    }
}
impl Drop for BlockedOwner {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("owner-dropped"), b"dropped");
    }
}

#[test]
#[ignore = "isolated process entry point"]
fn fixture_entrypoint() {
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    let role = std::env::var(ROLE).unwrap();
    if role != "parent" {
        let _endpoint = (role == "owned").then(|| endpoint(&ipc_path(&root)));
        fs::write(root.join(role), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(20));
        std::process::exit(9);
    }
    let mut command = ManagedMpvCommand::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", ENTRY, "--ignored", "--nocapture"])
        .env(ROLE, "owned")
        .env(ROOT, &root);
    #[cfg(windows)]
    let cleanup = None;
    #[cfg(unix)]
    let cleanup = Some(ipc_path(&root));
    let child = command.spawn(cleanup).unwrap();
    wait_marker(&root, "owned");
    let mut pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval_and_shutdown_timeout(
        GuiQueuedRuntimeBridgeHandle::default(),
        BlockedOwner {
            child,
            root: root.clone(),
        },
        Duration::from_millis(5),
        Duration::from_millis(300),
    )
    .unwrap();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    pump.pump(&state);
    wait_marker(&root, "owner-blocked");
    wait_marker(&root, "exit-parent");
    let started = Instant::now();
    drop(pump);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        !root.join("owner-dropped").exists(),
        "fixture must never release the worker"
    );
    // Reuse is tested before parent exit too: kernel parent-death containment
    // alone would not satisfy the GUI's independent shutdown contract.
    let _reused = endpoint(&ipc_path(&root));
    fs::write(
        root.join("shutdown-observed"),
        b"child exited; IPC reused; owner still blocked",
    )
    .unwrap();
    drop(_reused);
    #[cfg(unix)]
    fs::remove_file(ipc_path(&root)).unwrap();
    std::process::exit(0);
}

#[test]
fn gui_blocked_owner_parent_exit_terminates_owned_player_and_preserves_external_player() {
    let root = std::env::temp_dir().join(format!(
        "sorotte-gui-blocked-owner-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let mut external_player = external(&root, "external");
    let mut parent = external(&root, "parent");
    wait_marker(&root, "external");
    wait_marker(&root, "owner-blocked");
    fs::write(root.join("exit-parent"), b"exit now").unwrap();
    wait_until(
        Duration::from_secs(2),
        "isolated blocked owner parent exit",
        || parent.try_wait().unwrap().is_some(),
    );
    assert!(
        parent.wait().unwrap().success(),
        "isolated parent must verify bounded child cleanup"
    );
    assert!(root.join("shutdown-observed").exists());
    assert!(!root.join("owner-dropped").exists());
    assert!(
        external_player.try_wait().unwrap().is_none(),
        "external player must survive GUI exit"
    );
    let reused = endpoint(&ipc_path(&root));
    drop(reused);
    external_player.kill().unwrap();
    external_player.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
}
