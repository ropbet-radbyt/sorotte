use std::{
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sorotte-cli-storage-read-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("install")).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn wait_for_output(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut timed_out = false;
        while self
            .0
            .as_mut()
            .unwrap()
            .try_wait()
            .expect("CLI status should be readable")
            .is_none()
        {
            if Instant::now() >= deadline {
                timed_out = true;
                let child = self.0.as_mut().unwrap();
                // This invocation has no connection/player arguments. Its only
                // child is the owned CLI; kill and reap it before draining pipes.
                let _ = child.kill();
                child.wait().expect("timed-out CLI should be reaped");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // The malformed-locator invocation emits only a short startup error;
        // draining after confirmed termination cannot wait on its normal work.
        let output = self
            .0
            .take()
            .unwrap()
            .wait_with_output()
            .expect("CLI output should be captured");
        assert!(
            !timed_out,
            "CLI storage-resolution fixture exceeded 10s: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn unreadable_install_locator_stops_cli_before_default_settings_bootstrap() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("install/sorotte.ini"), [0xff]).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_sorotte-cli"));
    // Isolate storage and connection policy without changing this test process's environment.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SOROTTE_") {
            command.env_remove(name);
        }
    }
    let child = command
        .args(["--no-store", "--no-gui"])
        .env("SOROTTE_CLIENT_INSTALL_ROOT", fixture.0.join("install"))
        .env("APPDATA", fixture.0.join("default"))
        .env("HOME", fixture.0.join("default"))
        .env("XDG_CONFIG_HOME", fixture.0.join("default"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CLI fixture should launch");
    let output = OwnedChild(Some(child)).wait_for_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "read failure must reach process exit: {stderr}"
    );
    assert!(
        stderr.contains("failed reading install config locator"),
        "{stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("bootstrap complete"));
    assert!(
        !fixture.0.join("default").exists(),
        "failed locator must not select or create default storage"
    );
}
