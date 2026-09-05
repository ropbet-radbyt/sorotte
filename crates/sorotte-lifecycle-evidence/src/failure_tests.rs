use super::*;

fn observation() -> TransitionObservation {
    TransitionObservation::new(
        ProcessRole::Client,
        "application-process",
        "application",
        "APP-LAUNCH-001",
    )
}

#[test]
fn ignored_validation_failure_remains_visible_at_finalization() {
    let path = tests::unique_path("sticky-validation");
    let recorder = tests::recorder(&path);
    let error = recorder
        .emit(TransitionObservation::new(
            ProcessRole::Client,
            "private URL https://example.invalid/canary",
            "application",
            "APP-LAUNCH-001",
        ))
        .expect_err("invalid observation must fail");
    assert!(!error.to_string().contains("canary"));
    let result = recorder.flush();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.is_err(),
        "finalization must retain ignored validation failures"
    );
}

#[derive(Clone, Copy)]
enum Fault {
    AfterBytes(usize),
    Newline,
    Flush,
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
    fault: Option<Fault>,
}

#[derive(Clone, Default)]
struct FaultWriter(Arc<Mutex<Sink>>);

impl Write for FaultWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut sink = self.0.lock().unwrap();
        sink.writes += 1;
        let limit = match sink.fault {
            Some(Fault::AfterBytes(0)) => {
                sink.fault = None; // The fault is transient: future I/O would succeed.
                return Err(std::io::Error::other("injected write failure"));
            }
            Some(Fault::AfterBytes(count)) => {
                let written = bytes.len().min(count);
                sink.fault = Some(Fault::AfterBytes(count - written));
                written
            }
            Some(Fault::Newline) => {
                let written = bytes
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .unwrap_or(bytes.len());
                if written < bytes.len() {
                    sink.fault = Some(Fault::AfterBytes(0));
                }
                written
            }
            _ => bytes.len(),
        };
        sink.bytes.extend_from_slice(&bytes[..limit]);
        Ok(limit)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut sink = self.0.lock().unwrap();
        sink.flushes += 1;
        if matches!(sink.fault, Some(Fault::Flush)) {
            sink.fault = None;
            return Err(std::io::Error::other("injected flush failure"));
        }
        Ok(())
    }
}

fn with_writer(writer: FaultWriter) -> LifecycleEvidenceRecorder {
    LifecycleEvidenceRecorder::with_writer(
        writer,
        "run-001",
        "client-a",
        ProcessInventorySpec::new(
            ProcessRole::Client,
            [ProcessRole::Client, ProcessRole::Player],
        )
        .unwrap(),
        "a".repeat(64),
    )
    .unwrap()
}

#[test]
fn transient_faults_before_during_newline_and_flush_never_recover_health() {
    for fault in [
        Fault::AfterBytes(0),
        Fault::AfterBytes(17),
        Fault::Newline,
        Fault::Flush,
    ] {
        let writer = FaultWriter::default();
        let recorder = with_writer(writer.clone());
        recorder.emit(observation()).unwrap(); // A valid prefix has already been flushed.
        writer.0.lock().unwrap().fault = Some(fault);
        assert!(matches!(
            recorder.emit(observation()),
            Err(EvidenceError::Io(_))
        ));
        let snapshot = {
            let sink = writer.0.lock().unwrap();
            (sink.bytes.len(), sink.writes, sink.flushes)
        };
        let first = recorder.flush().unwrap_err().to_string();
        for _ in 0..3 {
            assert_eq!(recorder.emit(observation()).unwrap_err().to_string(), first);
            assert_eq!(recorder.flush().unwrap_err().to_string(), first);
        }
        let sink = writer.0.lock().unwrap();
        assert_eq!((sink.bytes.len(), sink.writes, sink.flushes), snapshot);
    }
}

#[test]
fn direct_final_flush_failure_is_also_sticky() {
    let writer = FaultWriter::default();
    let recorder = with_writer(writer.clone());
    writer.0.lock().unwrap().fault = Some(Fault::Flush);
    assert!(matches!(recorder.flush(), Err(EvidenceError::Io(_))));
    assert!(matches!(
        recorder.flush().unwrap_err().recording_failure(),
        Some(RecordingFailure::RecordingFailed { .. })
    ));
    assert!(recorder.emit(observation()).is_err());
}

#[test]
fn serialization_finishes_within_the_budget_before_any_external_write() {
    let mut writer = FaultWriter::default();
    let error = write_record(&mut writer, &"\u{0001}".repeat(MAX_RECORD_BYTES)).unwrap_err();
    assert!(matches!(
        error.recording_failure(),
        Some(RecordingFailure::RecordTooLarge)
    ));
    assert_eq!(writer.0.lock().unwrap().writes, 0);
    write_record(&mut writer, &"x".repeat(MAX_RECORD_BYTES - 3)).unwrap();
    assert_eq!(writer.0.lock().unwrap().bytes.len(), MAX_RECORD_BYTES);
    let mut writer = FaultWriter::default();

    struct Invalid;
    impl Serialize for Invalid {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("injected serialization failure"))
        }
    }
    assert!(matches!(
        write_record(&mut writer, &Invalid),
        Err(EvidenceError::Json(_))
    ));
    assert_eq!(writer.0.lock().unwrap().writes, 0);
}

#[test]
fn inventory_creation_reports_transient_writer_failure() {
    let writer = FaultWriter::default();
    writer.0.lock().unwrap().fault = Some(Fault::AfterBytes(0));
    let result = LifecycleEvidenceRecorder::with_writer(
        writer.clone(),
        "run-001",
        "client-a",
        ProcessInventorySpec::new(ProcessRole::Client, [ProcessRole::Client]).unwrap(),
        "a".repeat(64),
    );
    assert!(matches!(result, Err(EvidenceError::Io(_))));
    assert!(writer.0.lock().unwrap().bytes.is_empty());
}

#[test]
fn maximum_observation_fits_and_exhausted_sequence_never_wraps() {
    let writer = FaultWriter::default();
    let recorder = with_writer(writer.clone());
    let token = "a".repeat(MAX_TOKEN_LEN);
    let mut large = TransitionObservation::new(ProcessRole::Client, &token, &token, &token)
        .authority(&token, &token)
        .effect(&token, &token)
        .deadline(u64::MAX, true);
    for index in 0..MAX_IDENTITIES {
        large = large.identity(
            format!("{index:02}{}", "a".repeat(MAX_TOKEN_LEN - 2)),
            u64::MAX,
        );
    }
    for index in 0..MAX_PREDECESSORS - 1 {
        large = large.causal_predecessor(format!("{index:02}{}", "a".repeat(MAX_TOKEN_LEN - 2)));
    }
    recorder.emit(large).unwrap();
    recorder.inner.lock().unwrap().sequence = MAX_SEQUENCE;
    assert!(recorder.emit(observation()).unwrap().ends_with(".99999999"));
    assert!(matches!(
        recorder
            .emit(observation())
            .unwrap_err()
            .recording_failure(),
        Some(RecordingFailure::SequenceExhausted)
    ));
    assert!(
        recorder
            .flush()
            .unwrap_err()
            .to_string()
            .contains("exhausted")
    );
}

#[test]
fn concurrent_emitters_observe_one_sticky_failure_without_further_writes() {
    let writer = FaultWriter::default();
    let recorder = with_writer(writer.clone());
    let before = writer.0.lock().unwrap().writes;
    writer.0.lock().unwrap().fault = Some(Fault::AfterBytes(0));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let recorder = recorder.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                recorder.emit(observation()).unwrap_err()
            })
        })
        .collect::<Vec<_>>();
    let errors = handles
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(error, EvidenceError::Io(_))
                && error.recording_failure().is_none())
            .count(),
        1
    );
    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(
                error.recording_failure(),
                Some(RecordingFailure::RecordingFailed { .. })
            ))
            .count(),
        7
    );
    assert_eq!(writer.0.lock().unwrap().writes, before + 1);
    let first = recorder.flush().unwrap_err().to_string();
    for error in errors.iter().filter(|error| {
        matches!(
            error.recording_failure(),
            Some(RecordingFailure::RecordingFailed { .. })
        )
    }) {
        assert_eq!(error.to_string(), first);
    }
}

#[test]
fn invalid_roles_and_tokens_are_not_written_or_retained_in_health() {
    for invalid in [
        TransitionObservation::new(
            ProcessRole::Oracle,
            "private-canary",
            "application",
            "APP-LAUNCH-001",
        ),
        TransitionObservation::new(
            ProcessRole::Client,
            "https://private-canary.invalid",
            "application",
            "APP-LAUNCH-001",
        ),
        observation().identity("https://private-canary.invalid", 1),
        observation().causal_predecessor("private canary"),
    ] {
        let writer = FaultWriter::default();
        let recorder = with_writer(writer.clone());
        let before = writer.0.lock().unwrap().bytes.len();
        let error = recorder.emit(invalid).unwrap_err();
        assert!(!format!("{error:?}").contains("canary"));
        assert!(!format!("{:?}", recorder.flush().unwrap_err()).contains("canary"));
        assert_eq!(writer.0.lock().unwrap().bytes.len(), before);
    }
}

#[test]
fn automatic_predecessor_overflow_is_sticky_without_a_partial_record() {
    let writer = FaultWriter::default();
    let recorder = with_writer(writer.clone());
    let mut transition = observation();
    for index in 0..MAX_PREDECESSORS {
        transition = transition.causal_predecessor(format!("other.{index:08}"));
    }
    assert!(matches!(
        recorder.emit(transition),
        Err(EvidenceError::TooManyCausalPredecessors { .. })
    ));
    assert!(recorder.flush().is_err());
    assert_eq!(writer.0.lock().unwrap().writes, 1);
}

#[test]
fn product_writer_faults_reach_the_python_lifecycle_consumer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for fault in [
        None,
        Some(Fault::AfterBytes(0)),
        Some(Fault::AfterBytes(19)),
    ] {
        let writer = FaultWriter::default();
        let recorder = with_writer(writer.clone());
        recorder.emit(observation()).unwrap();
        writer.0.lock().unwrap().fault = fault;
        let emitted = recorder.emit(TransitionObservation::new(
            ProcessRole::Player,
            "attached-player",
            "application",
            "APP-RUN-001",
        ));
        let finalized = recorder.flush();
        assert_eq!(emitted.is_err(), fault.is_some());
        assert_eq!(finalized.is_err(), fault.is_some());
        let path = tests::unique_path("python-consumer");
        std::fs::write(&path, &writer.0.lock().unwrap().bytes).unwrap();
        let output = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg(root.join("scripts/playback_lifecycle_evidence.py"))
            .arg("--model")
            .arg(root.join("coverage/playback-lifecycle.toml"))
            .arg("--input")
            .arg(&path)
            .args(["--require-role", "client", "--require-role", "player"])
            .args(["--expected-digest", &format!("client-a={}", "a".repeat(64))])
            .output()
            .expect("Python is required for the producer-to-consumer regression");
        std::fs::remove_file(path).unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("consumer returns JSON status");
        assert_eq!(output.status.success(), fault.is_none(), "{report}");
        assert_eq!(
            report["result"],
            if fault.is_some() { "failed" } else { "passed" }
        );
    }
}
