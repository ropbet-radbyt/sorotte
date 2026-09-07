use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::legacy_settings::StoredClientSettingsMvp;

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sorotte-settings-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("sorotte.ini")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Worker(Child);
impl Worker {
    fn start(path: &std::path::Path, mode: &str, listener: &TcpListener) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sorotte_ini::transaction_tests::settings_writer_process_fixture",
                    "--nocapture",
                ])
                .env("SOROTTE_SETTINGS_FIXTURE_PATH", path)
                .env("SOROTTE_SETTINGS_FIXTURE_MODE", mode)
                .env(
                    "SOROTTE_SETTINGS_FIXTURE_CONTROL",
                    listener.local_addr().unwrap().to_string(),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    fn finish(mut self) {
        assert!(
            self.0.wait().unwrap().success(),
            "settings fixture child failed"
        );
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn receive(stream: &mut BufReader<TcpStream>, expected: &str) {
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), expected);
}
fn control(listener: &TcpListener) -> BufReader<TcpStream> {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            result => panic!("fixture did not connect within its deadline: {result:?}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    BufReader::new(stream)
}

#[test]
fn settings_writer_process_fixture() {
    let Ok(path) = std::env::var("SOROTTE_SETTINGS_FIXTURE_PATH") else {
        return;
    };
    let mode = std::env::var("SOROTTE_SETTINGS_FIXTURE_MODE").unwrap();
    let stream =
        TcpStream::connect(std::env::var("SOROTTE_SETTINGS_FIXTURE_CONTROL").unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut output = stream;
    if mode == "second" || mode == "conflicting" {
        let mut progress = output.try_clone().unwrap();
        super::transaction::CONTENTION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                writeln!(progress, "waiting").unwrap();
            }));
        });
    }
    update_sorotte_ini_stored_client_settings_mvp_at_path(
        std::path::Path::new(&path),
        |settings| {
            writeln!(output, "entered").unwrap();
            if mode == "first" {
                receive(&mut reader, "release");
                settings.username = Some("first-writer".into());
            } else if mode == "conflicting" {
                settings.username = Some("second-writer".into());
            } else {
                settings.room = Some("second-writer".into());
            }
        },
    )
    .unwrap();
    writeln!(output, "committed").unwrap();
}

#[test]
fn two_process_updates_wait_before_reading_and_preserve_disjoint_changes() {
    run_two_writers("second");
}

#[test]
fn two_process_conflicting_updates_follow_transaction_commit_order() {
    run_two_writers("conflicting");
}

fn run_two_writers(mode: &str) {
    let fixture = Fixture::new("two-process");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let first = Worker::start(&fixture.path(), "first", &listener);
    let mut first_control = control(&listener);
    receive(&mut first_control, "entered");
    let second = Worker::start(&fixture.path(), mode, &listener);
    let mut second_control = control(&listener);
    receive(&mut second_control, "waiting");
    // The external controller releases writer one; writer two cannot execute its
    // callback until writer one's entire read-modify-replace transaction ends.
    writeln!(first_control.get_mut(), "release").unwrap();
    receive(&mut first_control, "committed");
    receive(&mut second_control, "entered");
    receive(&mut second_control, "committed");
    first.finish();
    second.finish();
    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&fixture.path())
        .unwrap()
        .unwrap();
    if mode == "conflicting" {
        assert_eq!(settings.username.as_deref(), Some("second-writer"));
    } else {
        assert_eq!(settings.username.as_deref(), Some("first-writer"));
        assert_eq!(settings.room.as_deref(), Some("second-writer"));
    }
}

#[test]
fn crashed_process_releases_kernel_lock_without_removing_the_sidecar() {
    let fixture = Fixture::new("crash");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut first = Worker::start(&fixture.path(), "first", &listener);
    let mut first_control = control(&listener);
    receive(&mut first_control, "entered");
    first.0.kill().unwrap();
    first.0.wait().unwrap();
    let second = Worker::start(&fixture.path(), "after-crash", &listener);
    let mut second_control = control(&listener);
    receive(&mut second_control, "entered");
    receive(&mut second_control, "committed");
    second.finish();
    assert!(fixture.0.join(".sorotte.ini.lock").is_file());
}

#[test]
fn canonical_aliases_share_the_lock_and_report_a_bounded_busy_error() {
    let fixture = Fixture::new("aliases");
    let _guard = super::transaction::SettingsTransaction::acquire(&fixture.path()).unwrap();
    let alias = fixture.0.join(".").join("sorotte.ini");
    let error =
        super::transaction::SettingsTransaction::acquire_with_timeout(&alias, Duration::ZERO)
            .err()
            .expect("alias must contend");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(error.to_string().contains("busy"));
}

#[test]
fn callback_panic_releases_lock_and_a_retry_invokes_a_new_callback_once() {
    let fixture = Fixture::new("callback-panic");
    std::fs::write(fixture.path(), "[client_settings]\nname=before\n").unwrap();
    let result = std::panic::catch_unwind(|| {
        let _ = update_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), |_| {
            panic!("injected callback failure")
        });
    });
    assert!(result.is_err());
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&fixture.path())
            .unwrap()
            .unwrap()
            .username
            .as_deref(),
        Some("before")
    );
    let mut calls = 0;
    update_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), |settings| {
        calls += 1;
        settings.username = Some("after".into());
    })
    .unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn initial_snapshot_can_be_saved_but_clear_of_a_missing_file_fences_that_snapshot() {
    let fixture = Fixture::new("initial-clear");
    let snapshot = StoredClientSettingsMvp {
        username: Some("initial".into()),
        plex_user_token: Some("synthetic-token".into()),
        ..Default::default()
    };
    let initial =
        merge_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), &snapshot, &snapshot)
            .unwrap();
    assert_eq!(initial, snapshot);
    clear_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path()).unwrap();
    assert!(!clear_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path()).unwrap());
    let stale =
        merge_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), &snapshot, &snapshot)
            .unwrap();
    assert!(stale.plex_user_token.is_none());
    assert!(stale.username.is_none());
}

#[test]
fn open_readers_keep_original_document_when_settings_are_replaced() {
    use std::io::Read;

    let fixture = Fixture::new("open-reader");
    let before = format!("[unknown]\nvalue={}\n", "a".repeat(32768));
    let after = format!("[unknown]\nvalue={}\n", "b".repeat(32768));
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), before.as_bytes()).unwrap();
    let mut reader = std::fs::File::open(fixture.path()).unwrap();

    // The old handle remains open until after publication. Replacement must
    // complete independently of readers, while new opens see the new document.
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), after.as_bytes()).unwrap();
    assert_eq!(std::fs::read_to_string(fixture.path()).unwrap(), after);
    let mut observed = String::new();
    reader.read_to_string(&mut observed).unwrap();
    assert_eq!(observed, before);
}

#[test]
fn cooperating_readers_observe_complete_documents_through_repeated_replacement() {
    assert_complete_documents_during_replacement(|path| {
        super::read_sorotte_ini_contents_consistently_at_path(path)
            .unwrap()
            .expect("a cooperating replacement must not appear missing")
    });
}

#[cfg(not(windows))]
#[test]
fn readers_observe_complete_documents_through_repeated_atomic_replacement() {
    assert_complete_documents_during_replacement(|path| std::fs::read_to_string(path).unwrap());
}

#[cfg(windows)]
#[test]
#[ignore = "Windows NTFS raw concurrent opens can report missing during replacement; see microsoft/STL#5501"]
fn windows_raw_filesystem_readers_observe_complete_documents_through_replacement() {
    // Preserve the original strict OS probe. A passing run is not proof that the
    // known namespace gap is fixed; application readers use the sidecar lock.
    assert_complete_documents_during_replacement(|path| std::fs::read_to_string(path).unwrap());
}

fn assert_complete_documents_during_replacement(read: fn(&std::path::Path) -> String) {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };
    let fixture = Fixture::new("readers");
    let before = format!("[unknown]\nvalue={}\n", "a".repeat(32768));
    let after = format!("[unknown]\nvalue={}\n", "b".repeat(32768));
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), before.as_bytes()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(2));
    let reader = {
        let path = fixture.path();
        let stop = stop.clone();
        let start = start.clone();
        let before = before.clone();
        let after = after.clone();
        std::thread::spawn(move || {
            start.wait();
            let mut reads = 0;
            loop {
                let observed = read(&path);
                assert!(observed == before || observed == after);
                reads += 1;
                if stop.load(Ordering::Acquire) {
                    return reads;
                }
            }
        })
    };
    start.wait();
    let write_result = (0..20).try_for_each(|index| {
        write_sorotte_ini_contents_atomically_at_path(
            &fixture.path(),
            if index % 2 == 0 {
                after.as_bytes()
            } else {
                before.as_bytes()
            },
        )
    });
    stop.store(true, Ordering::Release);
    let read_result = reader.join();
    write_result.unwrap();
    assert!(read_result.unwrap() > 0);
}

#[cfg(unix)]
#[test]
fn unix_directory_alias_and_private_directory_mode_follow_the_contract() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new("unix-private");
    let private = fixture.0.join("private");
    create_private_directory(&private).unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(create_private_directory(&private).is_err());
    let alias = fixture.0.join("alias");
    symlink(&private, &alias).unwrap();
    assert!(create_private_directory(&alias).is_err());
    let _guard =
        super::transaction::SettingsTransaction::acquire(&private.join("sorotte.ini")).unwrap();
    assert!(
        super::transaction::SettingsTransaction::acquire_with_timeout(
            &alias.join("sorotte.ini"),
            Duration::ZERO
        )
        .is_err()
    );
}

#[test]
fn stale_full_snapshot_preserves_new_values_and_cannot_restore_cleared_secrets() {
    let fixture = Fixture::new("stale");
    let baseline = StoredClientSettingsMvp {
        username: Some("before".into()),
        server_password: Some("synthetic-password".into()),
        plex_user_token: Some("synthetic-user-token".into()),
        plex_selected_server_token: Some("synthetic-server-token".into()),
        ..Default::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), &baseline).unwrap();
    edit_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), |settings| {
        settings.server_password = None;
        settings.plex_user_token = None;
        settings.plex_selected_server_token = None;
        settings.room = Some("independent-room".into());
    })
    .unwrap();
    let mut desired = baseline.clone();
    desired.username = Some("after".into());
    let saved =
        merge_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), &baseline, &desired)
            .unwrap();
    assert_eq!(saved.username, desired.username);
    assert_eq!(saved.room.as_deref(), Some("independent-room"));
    assert!(saved.server_password.is_none());
    assert!(saved.plex_user_token.is_none());
    assert!(saved.plex_selected_server_token.is_none());
    clear_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path()).unwrap();
    let saved =
        merge_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path(), &baseline, &desired)
            .unwrap();
    assert!(saved.server_password.is_none());
    assert!(saved.plex_user_token.is_none());
    assert!(saved.plex_selected_server_token.is_none());
}

#[test]
fn relocation_uses_current_source_and_rolls_back_failed_publication() {
    let fixture = Fixture::new("relocation");
    let source = fixture.path();
    let destination = fixture.0.join("destination.ini");
    let baseline = StoredClientSettingsMvp {
        username: Some("old".into()),
        plex_user_token: Some("synthetic-token".into()),
        ..Default::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&source, &baseline).unwrap();
    edit_sorotte_ini_stored_client_settings_mvp_at_path(&source, |settings| {
        settings.plex_user_token = None;
    })
    .unwrap();
    std::fs::write(&destination, "; destination comment\n[unknown]\nkeep=yes\n").unwrap();
    let before = std::fs::read(&destination).unwrap();
    let mut desired = baseline.clone();
    desired.username = Some("new".into());
    let failed = relocate_sorotte_ini_stored_client_settings_mvp_at_path(
        Some(&source),
        &destination,
        &baseline,
        &desired,
        || anyhow::bail!("publication failed"),
    );
    assert!(failed.is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), before);
    let saved = relocate_sorotte_ini_stored_client_settings_mvp_at_path(
        Some(&source),
        &destination,
        &baseline,
        &desired,
        || Ok(()),
    )
    .unwrap();
    assert!(saved.plex_user_token.is_none());
    assert_eq!(saved.username.as_deref(), Some("new"));
    assert!(
        std::fs::read_to_string(&destination)
            .unwrap()
            .contains("[unknown]\nkeep=yes")
    );
}
