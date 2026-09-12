use super::*;
use std::cell::Cell;
use std::sync::mpsc;

#[test]
fn busy_executable_launches_after_the_writer_closes() {
    let root = FixtureRoot::new();
    let executable = root.wrapper("busy", "duration");
    let writer = fs::OpenOptions::new()
        .write(true)
        .open(&executable)
        .unwrap();
    let deadline = Deadline::after(Duration::from_secs(5));
    let started_at = Instant::now();
    let (tx, rx) = mpsc::channel();
    let (child, stdout, stderr) = thread::scope(|scope| {
        let launch = scope.spawn(|| {
            OwnedTool::spawn("ffprobe", &mut command(&executable, []), || {
                tx.send(()).unwrap();
                deadline.check("ffprobe", None)
            })
        });
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // The writer blocks the first exec. A second checkpoint proves that
        // launch retries, without relying on a timed release or suite ordering.
        rx.recv_timeout(Duration::from_secs(2))
            .expect("busy executable should be retried before the writer closes");
        assert!(!root.marker("busy").exists());
        drop(writer);
        launch.join().unwrap().unwrap()
    });
    let mut output = Vec::new();
    let (status, _, _) = run_started(
        "ffprobe",
        StartedTool {
            child,
            stdout,
            stderr,
            started_at,
        },
        None,
        deadline,
        PROBE_STDOUT_LIMIT,
        |chunk| {
            output.extend_from_slice(chunk);
            Ok(())
        },
    )
    .unwrap();
    assert!(status.success());
    assert!(String::from_utf8(output).unwrap().contains("\n120\n"));
    assert!(root.marker("busy").exists());
}

#[test]
fn busy_executable_cannot_outlast_the_operation_deadline() {
    let root = FixtureRoot::new();
    let executable = root.wrapper("busy", "duration");
    let _writer = fs::OpenOptions::new()
        .write(true)
        .open(&executable)
        .unwrap();
    let started_at = Instant::now();
    let result = run_output(
        "ffprobe",
        command(&executable, []),
        None,
        Deadline::after(Duration::from_millis(100)),
        PROBE_STDOUT_LIMIT,
    );
    assert!(
        matches!(
            result,
            Err(MediaFingerprintError::TimedOut {
                tool: "ffprobe",
                ..
            })
        ),
        "{result:?}"
    );
    assert!(started_at.elapsed() < Duration::from_secs(2));
    assert!(!root.marker("busy").exists());
}

#[test]
fn cancellation_during_a_busy_launch_starts_no_process() {
    let root = FixtureRoot::new();
    let executable = root.wrapper("busy", "duration");
    let _writer = fs::OpenOptions::new()
        .write(true)
        .open(&executable)
        .unwrap();
    let cancel = AtomicBool::new(false);
    let attempts = Cell::new(0);
    let deadline = Deadline::after(Duration::from_secs(5));
    let result = OwnedTool::spawn("ffprobe", &mut command(&executable, []), || {
        attempts.set(attempts.get() + 1);
        if attempts.get() == 2 {
            cancel.store(true, Ordering::Release);
        }
        deadline.check("ffprobe", Some(&cancel))
    });
    assert!(matches!(
        result,
        Err(MediaFingerprintError::Cancelled { tool: "ffprobe" })
    ));
    assert_eq!(attempts.get(), 2);
    assert!(!root.marker("busy").exists());
}

#[test]
fn missing_executable_fails_without_retrying() {
    let root = FixtureRoot::new();
    let attempts = Cell::new(0);
    let result = OwnedTool::spawn("ffprobe", &mut command(&root.marker("missing"), []), || {
        attempts.set(attempts.get() + 1);
        Ok(())
    });
    assert!(matches!(
        result,
        Err(MediaFingerprintError::ToolFailed { .. })
    ));
    assert_eq!(attempts.get(), 1);
}
