#![cfg(windows)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use sha2::{Digest, Sha256};
use zip::{ZipWriter, write::SimpleFileOptions};

const GUI_EXE: &str = "sorotte-gui.exe";
const UPDATER_EXE: &str = "sorotte-gui-updater.exe";
const INSTALL_MANIFEST: &str = "sorotte-install.json";
const ALLOW_ELEVATED_TEST_ENV: &str = "SOROTTE_UPDATER_INTEGRATION_TEST_ALLOW_ELEVATED";

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sorotte-running-updater-replacement-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    root
}

fn install_manifest(version: &str, gui: &[u8], updater: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "sorotte-gui-install-manifest-v2",
        "app": "sorotte-gui",
        "version": version,
        "target": "windows-x86_64",
        "files": [
            { "path": GUI_EXE, "sha256": sha256(gui) },
            { "path": UPDATER_EXE, "sha256": sha256(updater) }
        ]
    }))
    .expect("install manifest should serialize")
}

fn write_package(path: &Path, gui: &[u8], updater: &[u8]) {
    let file = fs::File::create(path).expect("update package should be created");
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    let manifest = install_manifest("0.2.5", gui, updater);
    for (name, bytes) in [
        (GUI_EXE, gui),
        (UPDATER_EXE, updater),
        (INSTALL_MANIFEST, manifest.as_slice()),
    ] {
        zip.start_file(name, options)
            .expect("package entry should start");
        zip.write_all(bytes).expect("package entry should write");
    }
    zip.finish().expect("update package should finish");
}

#[test]
fn running_installed_updater_can_replace_its_own_installed_path() {
    let root = test_root();
    let target = root.join("install");
    fs::create_dir_all(&target).expect("simulated install should be created");

    let updater_binary = PathBuf::from(env!("CARGO_BIN_EXE_sorotte-gui-updater"));
    let old_updater = fs::read(&updater_binary).expect("built updater should be readable");
    let old_gui = old_updater.clone();
    fs::write(target.join(UPDATER_EXE), &old_updater).expect("installed updater should be copied");
    fs::write(target.join(GUI_EXE), &old_gui).expect("old GUI should be written");
    fs::write(
        target.join(INSTALL_MANIFEST),
        install_manifest("0.2.4", &old_gui, &old_updater),
    )
    .expect("old manifest should be written");

    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot should be available");
    let new_gui = fs::read(PathBuf::from(&system_root).join("System32/cmd.exe"))
        .expect("replacement GUI fixture should be readable");
    let new_updater = fs::read(PathBuf::from(system_root).join("System32/where.exe"))
        .expect("replacement updater fixture should be readable");
    let package = root.join("update.zip");
    write_package(&package, &new_gui, &new_updater);
    let package_sha256 = sha256(&fs::read(&package).expect("package should be readable"));
    let log = root.join("update.log");
    let impossible_pid = u32::MAX.to_string();
    let package_arg = package.display().to_string();
    let target_arg = target.display().to_string();
    let log_arg = log.display().to_string();

    let status = Command::new(target.join(UPDATER_EXE))
        .env(ALLOW_ELEVATED_TEST_ENV, "1")
        .args([
            "--pid",
            &impossible_pid,
            "--package",
            &package_arg,
            "--package-sha256",
            &package_sha256,
            "--target-dir",
            &target_arg,
            "--target-exe",
            GUI_EXE,
            "--log",
            &log_arg,
        ])
        .status()
        .expect("the exact installed updater copy should launch");
    assert!(
        status.success(),
        "installed updater bootstrap should succeed"
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let gui_matches = fs::read(target.join(GUI_EXE)).is_ok_and(|bytes| bytes == new_gui);
        let updater_matches =
            fs::read(target.join(UPDATER_EXE)).is_ok_and(|bytes| bytes == new_updater);
        let transaction_finished = !target.join(".sorotte-update-journal-v1.jsonl").exists();
        if gui_matches && updater_matches && transaction_finished {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "detached updater did not replace both executables; log: {}",
            fs::read_to_string(&log).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(50));
    }

    assert_eq!(
        sha256(&fs::read(target.join(GUI_EXE)).unwrap()),
        sha256(&new_gui)
    );
    assert_eq!(
        sha256(&fs::read(target.join(UPDATER_EXE)).unwrap()),
        sha256(&new_updater)
    );
    assert!(!target.join(".sorotte-update-journal-v1.jsonl").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn running_installed_updater_recovers_interrupted_replacement_and_restarts() {
    let root = test_root();
    let target = root.join("install");
    fs::create_dir_all(&target).expect("simulated install should be created");
    let updater_binary = PathBuf::from(env!("CARGO_BIN_EXE_sorotte-gui-updater"));
    fs::copy(&updater_binary, target.join(UPDATER_EXE))
        .expect("installed updater should be copied");

    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot should be available");
    let original_gui = fs::read(PathBuf::from(&system_root).join("System32/where.exe"))
        .expect("original GUI fixture should be readable");
    let replacement_gui = fs::read(PathBuf::from(system_root).join("System32/cmd.exe"))
        .expect("replacement GUI fixture should be readable");
    let backup_name = format!(".{GUI_EXE}.sorotte-old-interrupted");
    let temporary_name = format!(".{GUI_EXE}.sorotte-new-interrupted");
    fs::write(target.join(GUI_EXE), &replacement_gui)
        .expect("interrupted replacement target should be written");
    fs::write(target.join(&backup_name), &original_gui).expect("rollback backup should be written");
    let journal = json!({
        "schema": "sorotte-update-replacement-journal-v1",
        "entries": [{
            "relative": GUI_EXE,
            "temporary": temporary_name,
            "backup": backup_name,
            "targetExisted": true,
            "originalSha256": sha256(&original_gui),
            "replacementSha256": sha256(&replacement_gui)
        }]
    });
    fs::write(
        target.join(".sorotte-update-journal-v1.jsonl"),
        format!("{}\n", serde_json::to_string(&journal).unwrap()),
    )
    .expect("interrupted journal should be written");
    let log = root.join("recovery.log");
    let impossible_pid = u32::MAX.to_string();
    let target_arg = target.display().to_string();
    let log_arg = log.display().to_string();

    let status = Command::new(target.join(UPDATER_EXE))
        .env(ALLOW_ELEVATED_TEST_ENV, "1")
        .args([
            "--recover",
            "--pid",
            &impossible_pid,
            "--target-dir",
            &target_arg,
            "--target-exe",
            GUI_EXE,
            "--log",
            &log_arg,
            "--restart",
        ])
        .status()
        .expect("installed recovery updater should launch");
    assert!(
        status.success(),
        "installed recovery bootstrap should succeed"
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rolled_back = fs::read(target.join(GUI_EXE)).is_ok_and(|bytes| bytes == original_gui);
        let finished = !target.join(".sorotte-update-journal-v1.jsonl").exists();
        let completion_logged = fs::read_to_string(&log)
            .is_ok_and(|body| body.contains("interrupted update recovery completed"));
        if rolled_back && finished && completion_logged {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "detached updater did not recover the interrupted transaction; log: {}",
            fs::read_to_string(&log).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(50));
    }

    let log_body = fs::read_to_string(&log).expect("recovery log should be readable");
    assert!(log_body.contains("restarting Sorotte GUI after update recovery"));
    assert!(log_body.contains("interrupted update recovery completed"));
    assert!(!target.join(&backup_name).exists());
    assert!(!target.join(&temporary_name).exists());
    let _ = fs::remove_dir_all(root);
}
