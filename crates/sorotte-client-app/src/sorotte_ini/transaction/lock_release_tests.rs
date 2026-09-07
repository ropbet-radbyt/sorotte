use std::{
    cell::RefCell,
    fs::File,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{ACQUIRED_HOOK, SettingsTransaction, read_consistently_with_timeout};

const BEFORE: &str = "[client_settings]\nname=before\n";
const AFTER: &str = "[client_settings]\nname=after\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sorotte-settings-lock-release-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0.join("sorotte.ini")
    }

    fn initialize(&self) {
        let writer = SettingsTransaction::acquire(&self.path()).unwrap();
        std::fs::write(writer.path(), BEFORE).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct CapturedDescriptor(Rc<RefCell<Option<File>>>);

impl CapturedDescriptor {
    fn assert_live(&self) {
        assert!(
            self.0
                .borrow()
                .as_ref()
                .expect("the real acquisition hook must have captured its descriptor")
                .metadata()
                .unwrap()
                .is_file()
        );
    }
}

impl Drop for CapturedDescriptor {
    fn drop(&mut self) {
        ACQUIRED_HOOK.with(|hook| drop(hook.borrow_mut().take()));
    }
}

fn capture_next_acquisition() -> CapturedDescriptor {
    let captured = Rc::new(RefCell::new(None));
    let destination = captured.clone();
    ACQUIRED_HOOK.with(|hook| {
        assert!(
            hook.borrow().is_none(),
            "an acquisition hook is already armed"
        );
        *hook.borrow_mut() = Some(Box::new(move |file| {
            *destination.borrow_mut() = Some(file.try_clone().unwrap());
        }));
    });
    CapturedDescriptor(captured)
}

fn read_bytes(path: &Path) -> anyhow::Result<Option<String>> {
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

fn assert_writer_busy(path: &Path) {
    assert_busy(
        SettingsTransaction::acquire_with_timeout(path, Duration::ZERO)
            .err()
            .expect("a live owner must exclude another writer"),
    );
}

fn assert_reader_busy(path: &Path) {
    assert_busy(
        read_consistently_with_timeout(path, Duration::ZERO, |_| {
            panic!("a blocked reader must not inspect the settings")
        })
        .unwrap_err(),
    );
}

fn assert_writer_available(path: &Path) {
    drop(SettingsTransaction::acquire_with_timeout(path, Duration::ZERO).unwrap());
}

#[test]
fn writer_scope_releases_lock_while_a_duplicate_descriptor_survives() {
    let fixture = Fixture::new("writer");
    fixture.initialize();
    let duplicate = capture_next_acquisition();
    let writer = SettingsTransaction::acquire(&fixture.path()).unwrap();
    duplicate.assert_live();
    assert_writer_busy(writer.path());
    assert_reader_busy(writer.path());
    drop(writer);

    duplicate.assert_live();
    assert_eq!(
        read_consistently_with_timeout(&fixture.path(), Duration::ZERO, read_bytes)
            .unwrap()
            .as_deref(),
        Some(BEFORE)
    );
    let next_writer =
        SettingsTransaction::acquire_with_timeout(&fixture.path(), Duration::ZERO).unwrap();
    drop(duplicate);
    assert_reader_busy(next_writer.path());
    assert_writer_busy(next_writer.path());
    drop(next_writer);
    assert_writer_available(&fixture.path());
}

#[test]
fn writer_validation_failure_releases_a_successfully_acquired_lock() {
    let fixture = Fixture::new("writer-validation");
    std::fs::create_dir(fixture.path()).unwrap();
    let duplicate = capture_next_acquisition();
    let error = SettingsTransaction::acquire_with_timeout(&fixture.path(), Duration::ZERO)
        .err()
        .expect("the invalid destination must fail after lock acquisition");
    assert!(
        error
            .to_string()
            .contains("stored settings path is not a file")
    );
    duplicate.assert_live();
    std::fs::remove_dir(fixture.path()).unwrap();
    std::fs::write(fixture.path(), BEFORE).unwrap();
    assert_writer_available(&fixture.path());
    duplicate.assert_live();
}

#[test]
fn writer_unwind_releases_lock_while_a_duplicate_descriptor_survives() {
    let fixture = Fixture::new("writer-panic");
    fixture.initialize();
    let duplicate = capture_next_acquisition();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let writer = SettingsTransaction::acquire(&fixture.path()).unwrap();
        duplicate.assert_live();
        assert_reader_busy(writer.path());
        panic!("injected writer panic");
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"injected writer panic"));
    duplicate.assert_live();
    assert_writer_available(&fixture.path());
}

#[test]
fn successful_read_releases_only_its_own_shared_lock() {
    let fixture = Fixture::new("read-success");
    fixture.initialize();
    let duplicate = capture_next_acquisition();
    let observed = read_consistently_with_timeout(&fixture.path(), Duration::ZERO, |path| {
        duplicate.assert_live();
        assert_writer_busy(path);
        assert_eq!(
            read_consistently_with_timeout(path, Duration::ZERO, read_bytes)?.as_deref(),
            Some(BEFORE)
        );
        // Releasing the nested reader must not release the independent outer reader.
        assert_writer_busy(path);
        read_bytes(path)
    })
    .unwrap();
    assert_eq!(observed.as_deref(), Some(BEFORE));
    duplicate.assert_live();
    assert_writer_available(&fixture.path());
}

#[test]
fn failed_read_releases_lock_without_replacing_its_primary_error() {
    let fixture = Fixture::new("read-error");
    fixture.initialize();
    let duplicate = capture_next_acquisition();
    let error = read_consistently_with_timeout(&fixture.path(), Duration::ZERO, |path| {
        duplicate.assert_live();
        assert_writer_busy(path);
        Err(io::Error::new(io::ErrorKind::InvalidData, "injected read error").into())
    })
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(error.to_string(), "injected read error");
    duplicate.assert_live();
    assert_writer_available(&fixture.path());
}

#[test]
fn unwinding_read_releases_lock_without_masking_its_panic() {
    let fixture = Fixture::new("read-panic");
    fixture.initialize();
    let duplicate = capture_next_acquisition();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = read_consistently_with_timeout(&fixture.path(), Duration::ZERO, |path| {
            duplicate.assert_live();
            assert_writer_busy(path);
            panic!("injected read panic");
        });
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"injected read panic"));
    duplicate.assert_live();
    assert_writer_available(&fixture.path());
}

#[test]
fn provisional_read_releases_the_lock_used_for_the_second_read() {
    let fixture = Fixture::new("provisional");
    std::fs::write(fixture.path(), BEFORE).unwrap();
    let mut calls = 0;
    let mut duplicate = None;
    let observed = read_consistently_with_timeout(&fixture.path(), Duration::ZERO, |path| {
        calls += 1;
        if calls == 1 {
            assert!(!fixture.0.join(".sorotte.ini.lock").exists());
            let provisional = read_bytes(path)?;
            let writer = SettingsTransaction::acquire(path)?;
            std::fs::write(writer.path(), AFTER)?;
            drop(writer);
            duplicate = Some(capture_next_acquisition());
            return Ok(provisional);
        }
        assert_eq!(calls, 2);
        duplicate.as_ref().unwrap().assert_live();
        assert_writer_busy(path);
        read_bytes(path)
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(observed.as_deref(), Some(AFTER));
    let duplicate = duplicate.unwrap();
    duplicate.assert_live();
    assert_writer_available(&fixture.path());
    duplicate.assert_live();
}
