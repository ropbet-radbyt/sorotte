use super::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{OnceLock, atomic::AtomicBool},
    time::Duration,
};

pub(in crate::app) fn tool_fixture(root: &Path, mode: &str) -> PathBuf {
    static COMPILED: OnceLock<tempfile::TempDir> = OnceLock::new();
    let compiled = COMPILED.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("fixture.rs");
        fs::write(&source, include_str!("tool_fixture.rs")).unwrap();
        let mut command = std::process::Command::new("rustc");
        command
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(root.path().join("fixture.exe"));
        crate::app::child_process::configure_gui_child_process(&mut command);
        let output = command
            .output()
            .expect("rustc builds the controlled helper fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        root
    });
    let target = root.join(format!("{mode}-fixture.exe"));
    fs::copy(compiled.path().join("fixture.exe"), &target).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&target).unwrap();
    writeln!(file, "\nSOROTTE_FIXTURE:{mode}").unwrap();
    target
}

#[test]
fn failed_second_component_restores_binary_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("one.exe"), b"old one").unwrap();
    fs::write(root.path().join("metadata.json"), b"old metadata").unwrap();
    let install = ToolInstall::begin(root.path(), None).unwrap();
    fs::write(install.path("one.exe"), b"new one").unwrap();
    install
        .write_metadata(&serde_json::json!({"new":true}))
        .unwrap();
    assert!(install.commit(&["one.exe", "missing.exe"], None).is_err());
    assert_eq!(fs::read(root.path().join("one.exe")).unwrap(), b"old one");
    assert_eq!(
        fs::read(root.path().join("metadata.json")).unwrap(),
        b"old metadata"
    );
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".install-stage")
    }));
}

#[test]
fn root_lock_and_cancel_preserve_existing_installation() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("one.exe"), b"old").unwrap();
    let install = ToolInstall::begin(root.path(), None).unwrap();
    assert!(ToolInstall::begin(root.path(), None).is_err());
    fs::write(install.path("one.exe"), b"new").unwrap();
    install.write_metadata(&()).unwrap();
    assert!(
        install
            .commit(&["one.exe"], Some(&AtomicBool::new(true)))
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("one.exe")).unwrap(), b"old");
    assert!(ToolInstall::begin(root.path(), None).is_ok());
}

#[test]
fn archive_requires_one_unambiguous_component() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("tools.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for name in ["first/bin/tool.exe", "second/bin/tool.exe"] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"executable").unwrap();
    }
    writer.finish().unwrap();
    let target = root.path().join("tool.exe");
    assert!(
        extract_executable(&archive, "tool.exe", &target, None)
            .unwrap_err()
            .contains("duplicate")
    );
    assert!(
        extract_executable(&archive, "absent.exe", &target, None)
            .unwrap_err()
            .contains("did not contain")
    );
    assert!(!target.exists());
}

#[test]
fn dropping_worker_signals_cancellation_without_waiting_for_progress() {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = HelperWorker::<()>::spawn("cancel-test", None, move |cancel, _| {
        while !cancel.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
        tx.send(()).unwrap();
    })
    .unwrap();
    drop(worker);
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
}
