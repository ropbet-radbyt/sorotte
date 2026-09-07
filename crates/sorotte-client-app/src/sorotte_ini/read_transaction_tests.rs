use std::{
    cell::RefCell,
    io::{self, BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::{
    clear_sorotte_ini_stored_client_settings_mvp_at_path, ensure_sorotte_ini_contents_at_path,
    load_sorotte_ini_stored_client_settings_mvp_from_path, on_next_settings_lock_contention,
    paths::write_sorotte_ini_contents_atomically_with_injected_pre_commit,
    read_sorotte_ini_contents_consistently_at_path,
    transaction::{SettingsTransaction, read_consistently_with_timeout},
    update_sorotte_ini_contents_at_path, write_sorotte_ini_contents_atomically_at_path,
};

const BEFORE: &str = "[client_settings]\nname=before\n";
const AFTER: &str = "[client_settings]\nname=after\n";

struct Fixture(PathBuf, RefCell<Vec<(PathBuf, std::fs::Permissions)>>);

impl Fixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sorotte-settings-read-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path, RefCell::new(Vec::new()))
    }

    fn path(&self) -> PathBuf {
        self.0.join("sorotte.ini")
    }

    fn lock(&self) -> PathBuf {
        self.0.join(".sorotte.ini.lock")
    }

    fn set_permissions(&self, path: &Path, permissions: std::fs::Permissions) {
        assert!(path.starts_with(&self.0));
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        self.1
            .borrow_mut()
            .push((path.to_path_buf(), metadata.permissions()));
        std::fs::set_permissions(path, permissions).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Restore exactly the permissions changed by this fixture, including
        // the parent before its children when the directory was made read-only.
        for (path, permissions) in self.1.get_mut().drain(..).rev() {
            let _ = std::fs::set_permissions(path, permissions);
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn actual_read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn assert_busy(error: anyhow::Error) {
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn missing_reads_do_not_create_parent_directories_or_sidecars() {
    let fixture = Fixture::new("missing");
    let nested = fixture.0.join("absent/nested/sorotte.ini");
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&nested).unwrap(),
        None
    );
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&fixture.path()).unwrap(),
        None
    );
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn legacy_readonly_document_and_ensure_leave_the_directory_unchanged() {
    let fixture = Fixture::new("legacy-readonly");
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let mut permissions = std::fs::metadata(fixture.path()).unwrap().permissions();
    permissions.set_readonly(true);
    fixture.set_permissions(&fixture.path(), permissions);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fixture.set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o500));
    }
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.path())
            .unwrap()
            .as_deref(),
        Some(BEFORE)
    );
    assert!(!ensure_sorotte_ini_contents_at_path(&fixture.path(), AFTER.as_bytes()).unwrap());
    assert!(!fixture.lock().exists());
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
    assert!(
        std::fs::metadata(fixture.path())
            .unwrap()
            .permissions()
            .readonly()
    );
}

#[test]
fn readonly_existing_sidecar_supports_shared_reads_without_mutation() {
    let fixture = Fixture::new("readonly-lock");
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), BEFORE.as_bytes()).unwrap();
    let before = std::fs::read(fixture.lock()).unwrap();
    let mut permissions = std::fs::metadata(fixture.lock()).unwrap().permissions();
    permissions.set_readonly(true);
    fixture.set_permissions(&fixture.lock(), permissions);
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.path())
            .unwrap()
            .as_deref(),
        Some(BEFORE)
    );
    assert_eq!(std::fs::read(fixture.lock()).unwrap(), before);
    assert!(
        std::fs::metadata(fixture.lock())
            .unwrap()
            .permissions()
            .readonly()
    );
}

#[test]
fn empty_document_is_present_and_clear_is_absent_without_removing_the_lock() {
    let fixture = Fixture::new("empty-clear");
    assert!(ensure_sorotte_ini_contents_at_path(&fixture.path(), b"").unwrap());
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.path())
            .unwrap()
            .as_deref(),
        Some("")
    );
    assert!(!ensure_sorotte_ini_contents_at_path(&fixture.path(), AFTER.as_bytes()).unwrap());
    assert!(clear_sorotte_ini_stored_client_settings_mvp_at_path(&fixture.path()).unwrap());
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.path()).unwrap(),
        None
    );
    assert_eq!(std::fs::metadata(fixture.lock()).unwrap().len(), 1);
}

#[test]
fn shared_readers_overlap_while_excluding_a_writer() {
    let fixture = Fixture::new("shared");
    write_sorotte_ini_contents_atomically_at_path(&fixture.path(), BEFORE.as_bytes()).unwrap();
    let observed = read_consistently_with_timeout(&fixture.path(), Duration::ZERO, |path| {
        assert_busy(
            SettingsTransaction::acquire_with_timeout(path, Duration::ZERO)
                .err()
                .unwrap(),
        );
        assert_eq!(
            read_consistently_with_timeout(path, Duration::ZERO, actual_read)?.as_deref(),
            Some(BEFORE)
        );
        actual_read(path)
    })
    .unwrap();
    assert_eq!(observed.as_deref(), Some(BEFORE));
    assert!(SettingsTransaction::acquire_with_timeout(&fixture.path(), Duration::ZERO).is_ok());
}

#[test]
fn first_sidecar_creation_discards_a_provisional_missing_read() {
    let fixture = Fixture::new("first-writer-race");
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let mut reads = 0;
    let observed =
        read_consistently_with_timeout(&fixture.path(), Duration::from_secs(2), |path| {
            reads += 1;
            if reads == 1 {
                assert!(!fixture.lock().exists());
                // The first writer starts after the reader's absent-sidecar check.
                // Expose an actual missing name under its exclusive lock, retain
                // that provisional read, and finish publication before rechecking.
                let writer = SettingsTransaction::acquire(path)?;
                std::fs::remove_file(writer.path())?;
                let provisional = actual_read(path)?;
                assert!(provisional.is_none());
                write_sorotte_ini_contents_atomically_with_injected_pre_commit(
                    writer.path(),
                    AFTER.as_bytes(),
                    |_| Ok(()),
                )?;
                return Ok(provisional);
            }
            actual_read(path)
        })
        .unwrap();
    assert_eq!(reads, 2);
    assert_eq!(observed.as_deref(), Some(AFTER));
}

#[test]
fn busy_reader_errors_before_reading_instead_of_returning_missing_settings() {
    let fixture = Fixture::new("busy");
    let _writer = SettingsTransaction::acquire(&fixture.path()).unwrap();
    let error =
        read_consistently_with_timeout(&fixture.0.join("./sorotte.ini"), Duration::ZERO, |_| {
            panic!("a busy reader must not inspect provisional settings")
        })
        .unwrap_err();
    assert_busy(error);
}

#[test]
fn malformed_document_and_invalid_sidecar_are_errors_not_absence() {
    let fixture = Fixture::new("errors");
    std::fs::write(fixture.path(), [0xff, 0xfe]).unwrap();
    let error = read_sorotte_ini_contents_consistently_at_path(&fixture.path()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    std::fs::remove_file(fixture.path()).unwrap();
    std::fs::create_dir(fixture.lock()).unwrap();
    assert!(read_sorotte_ini_contents_consistently_at_path(&fixture.path()).is_err());
}

#[test]
fn byte_update_failure_preserves_contents_and_releases_the_transaction() {
    let fixture = Fixture::new("byte-update");
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let mut calls = 0;
    let result = update_sorotte_ini_contents_at_path(&fixture.path(), |contents| {
        calls += 1;
        assert_eq!(contents, Some(BEFORE));
        anyhow::bail!("injected byte update failure")
    });
    assert!(result.is_err());
    assert_eq!(calls, 1);
    assert_eq!(std::fs::read_to_string(fixture.path()).unwrap(), BEFORE);
    update_sorotte_ini_contents_at_path(&fixture.path(), |contents| {
        Ok(format!("{}; appended\n", contents.unwrap()))
    })
    .unwrap();
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.path())
            .unwrap()
            .unwrap(),
        format!("{BEFORE}; appended\n")
    );
}

#[test]
fn non_ascii_settings_names_keep_the_same_sidecar_identity() {
    let fixture = Fixture::new("unicode");
    let path = fixture.0.join("settings-\u{e9}-\u{540c}.ini");
    write_sorotte_ini_contents_atomically_at_path(&path, BEFORE.as_bytes()).unwrap();
    let _writer = SettingsTransaction::acquire(&path).unwrap();
    assert_busy(read_consistently_with_timeout(&path, Duration::ZERO, actual_read).unwrap_err());
}

#[cfg(unix)]
#[test]
fn file_and_directory_symlink_readers_lock_the_target_even_while_it_is_missing() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("symlinks");
    let real = fixture.0.join("real");
    std::fs::create_dir(&real).unwrap();
    let path = real.join("sorotte.ini");
    write_sorotte_ini_contents_atomically_at_path(&path, BEFORE.as_bytes()).unwrap();
    symlink("real/sorotte.ini", fixture.0.join("file-alias.ini")).unwrap();
    symlink("real", fixture.0.join("directory-alias")).unwrap();
    let writer = SettingsTransaction::acquire(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    for alias in [
        fixture.0.join("file-alias.ini"),
        fixture.0.join("directory-alias/sorotte.ini"),
    ] {
        assert_busy(
            read_consistently_with_timeout(&alias, Duration::ZERO, actual_read).unwrap_err(),
        );
    }
    std::fs::write(&path, AFTER).unwrap();
    drop(writer);
    assert_eq!(
        read_sorotte_ini_contents_consistently_at_path(&fixture.0.join("file-alias.ini"))
            .unwrap()
            .as_deref(),
        Some(AFTER)
    );
    assert!(!fixture.0.join(".file-alias.ini.lock").exists());
}

#[cfg(windows)]
#[test]
fn windows_case_aliases_share_the_read_lock_and_relocation_identity() {
    let fixture = Fixture::new("case-alias");
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let alias = fixture.0.join("SOROTTE.INI");
    let writer = SettingsTransaction::acquire(&fixture.path()).unwrap();
    assert_eq!(
        super::transaction::canonical_settings_path(&alias).unwrap(),
        writer.path()
    );
    std::fs::remove_file(fixture.path()).unwrap();
    assert_busy(read_consistently_with_timeout(&alias, Duration::ZERO, actual_read).unwrap_err());
    std::fs::write(fixture.path(), BEFORE).unwrap();
    drop(writer);
    let before = super::parse_sorotte_ini_stored_client_settings_mvp(BEFORE);
    let after = super::parse_sorotte_ini_stored_client_settings_mvp(AFTER);
    super::relocate_sorotte_ini_stored_client_settings_mvp_at_path(
        Some(&alias),
        &fixture.path(),
        &before,
        &after,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&alias)
            .unwrap()
            .unwrap()
            .username
            .as_deref(),
        Some("after")
    );
}

struct ReaderProcess(Child);

impl ReaderProcess {
    fn start(path: &Path, address: std::net::SocketAddr) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sorotte_ini::read_transaction_tests::settings_reader_process_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("SOROTTE_SETTINGS_READER_PATH", path)
                .env("SOROTTE_SETTINGS_READER_CONTROL", address.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        )
    }

    fn finish(mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "settings reader child failed");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "settings reader child did not exit"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ReaderProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "settings reader subprocess entry point, invoked by cross-process reader contracts"]
fn settings_reader_process_fixture() {
    let path = std::env::var("SOROTTE_SETTINGS_READER_PATH")
        .expect("settings reader fixture requires its parent-owned settings path");
    let control = std::env::var("SOROTTE_SETTINGS_READER_CONTROL")
        .expect("settings reader fixture requires its parent-owned control endpoint");
    let mut stream = TcpStream::connect(control).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut progress = stream.try_clone().unwrap();
    on_next_settings_lock_contention(move || writeln!(progress, "waiting").unwrap());
    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(Path::new(&path)).unwrap();
    let username = settings.and_then(|settings| settings.username);
    writeln!(stream, "{}", serde_json::to_string(&username).unwrap()).unwrap();
}

fn run_blocked_process_reader(clear: bool) {
    let fixture = Fixture::new(if clear {
        "process-clear"
    } else {
        "process-replace"
    });
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let writer = SettingsTransaction::acquire(&fixture.path()).unwrap();
    if clear {
        writer.mark_cleared().unwrap();
    }
    std::fs::remove_file(fixture.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = ReaderProcess::start(&fixture.path(), listener.local_addr().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => panic!("reader process did not connect: {result:?}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut stream = BufReader::new(stream);
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    assert_eq!(
        line.trim(),
        "waiting",
        "reader must lock even when the destination name is absent"
    );
    if !clear {
        write_sorotte_ini_contents_atomically_with_injected_pre_commit(
            writer.path(),
            AFTER.as_bytes(),
            |_| Ok(()),
        )
        .unwrap();
    }
    drop(writer);
    line.clear();
    stream.read_line(&mut line).unwrap();
    let observed: Option<String> = serde_json::from_str(&line).unwrap();
    assert_eq!(
        observed.as_deref(),
        if clear { None } else { Some("after") }
    );
    worker.finish();
}

#[test]
fn cross_process_reader_waits_through_a_writer_owned_missing_name() {
    run_blocked_process_reader(false);
}

#[test]
fn cross_process_reader_observes_clear_only_after_the_writer_unlocks() {
    run_blocked_process_reader(true);
}
