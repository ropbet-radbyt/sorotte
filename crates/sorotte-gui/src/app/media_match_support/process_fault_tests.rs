use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::{
    MEDIA_MATCH_VERSION_CAPTURE_LIMIT_BYTES, MediaMatchTool, probe_executable_output_with_timeout,
    probe_executable_version,
};

const LARGE_STDOUT_FIXTURE_TEST: &str = concat!(
    "app::media_match_support::process_fault_tests::",
    "media_match_large_stdout_process_fixture"
);
const PARKED_FIXTURE_TEST: &str = concat!(
    "app::media_match_support::process_fault_tests::",
    "media_match_parked_process_fixture"
);
const FINITE_FAKE_TOOL_OUTPUT_BYTES: usize = 512 * 1024;
const PROCESS_FIXTURE_TIMEOUT: Duration = Duration::from_millis(400);
static PROCESS_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct FakeMediaMatchTool {
    root: PathBuf,
    executable: PathBuf,
}

impl FakeMediaMatchTool {
    fn new(case: &str) -> Self {
        let sequence = PROCESS_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sorotte-gui-media-match-process-{}-{sequence}-{case}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("fake media-match tool root should be created");

        let source = std::env::current_exe().expect("GUI unit-test image should exist");
        let extension = source
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        let executable = root.join(format!("fake-media-match-tool{extension}"));
        fs::hard_link(&source, &executable)
            .or_else(|_| fs::copy(&source, &executable).map(|_| ()))
            .expect("fake media-match tool image should be materialized");
        Self { root, executable }
    }

    fn executable(&self) -> &Path {
        &self.executable
    }

    fn assert_process_and_image_released(&self) {
        fs::remove_file(&self.executable)
            .expect("the reaped fake-tool process must release its executable image");
        assert!(!self.executable.exists());
    }
}

impl Drop for FakeMediaMatchTool {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn invoked_as_exact_fixture(test_name: &str) -> bool {
    let args = std::env::args().collect::<Vec<_>>();
    args.iter().any(|arg| arg == "--exact") && args.iter().any(|arg| arg == test_name)
}

fn fixture_args(test_name: &'static str) -> [&'static str; 3] {
    ["--exact", test_name, "--nocapture"]
}

#[cfg(windows)]
fn shell_probe(script: &str) -> Result<String, String> {
    probe_executable_version(
        Path::new("powershell.exe"),
        &["-NoProfile", "-NonInteractive", "-Command", script],
        MediaMatchTool::Ffprobe,
    )
}

#[cfg(not(windows))]
fn shell_probe(script: &str) -> Result<String, String> {
    probe_executable_version(
        Path::new("/bin/sh"),
        &["-c", script],
        MediaMatchTool::Ffprobe,
    )
}

#[cfg(windows)]
const COMPLETE_VERSION_SCRIPT: &str =
    "[Console]::Out.Write(\"`r`nffprobe version 8.0`r`nconfiguration details`r`n\")";
#[cfg(not(windows))]
const COMPLETE_VERSION_SCRIPT: &str = "printf '\\nffprobe version 8.0\\nconfiguration details\\n'";

#[cfg(windows)]
const UNTERMINATED_VERSION_SCRIPT: &str = "[Console]::Out.Write('ffprobe version 8.0-no-newline')";
#[cfg(not(windows))]
const UNTERMINATED_VERSION_SCRIPT: &str = "printf 'ffprobe version 8.0-no-newline'";

#[cfg(windows)]
const NONZERO_EXIT_SCRIPT: &str = "[Console]::Error.Write('probe failure'); exit 23";
#[cfg(not(windows))]
const NONZERO_EXIT_SCRIPT: &str = "printf 'probe failure' >&2; exit 23";

#[cfg(windows)]
const EMPTY_SUCCESS_SCRIPT: &str = "exit 0";
#[cfg(not(windows))]
const EMPTY_SUCCESS_SCRIPT: &str = "exit 0";

#[cfg(windows)]
const INVALID_UTF8_SCRIPT: &str = concat!(
    "$stdout=[Console]::OpenStandardOutput();",
    "$bytes=[byte[]](0x66,0x66,0xff,0x70,0x72,0x6f,0x62,0x65,0x0a);",
    "$stdout.Write($bytes,0,$bytes.Length)"
);
#[cfg(not(windows))]
const INVALID_UTF8_SCRIPT: &str = "printf 'ff\\377probe\\n'";

#[cfg(windows)]
const UNRELATED_SUCCESS_SCRIPT: &str = "[Console]::Out.Write('not a media tool')";
#[cfg(not(windows))]
const UNRELATED_SUCCESS_SCRIPT: &str = "printf 'not a media tool'";

#[test]
fn media_match_large_stdout_process_fixture() {
    if !invoked_as_exact_fixture(LARGE_STDOUT_FIXTURE_TEST) {
        return;
    }

    let chunk = [b'x'; 16 * 1024];
    let mut stdout = io::stdout().lock();
    for _ in 0..(FINITE_FAKE_TOOL_OUTPUT_BYTES / chunk.len()) {
        stdout
            .write_all(&chunk)
            .expect("large-output fixture should write stdout");
    }
    stdout
        .flush()
        .expect("large-output fixture should flush stdout");
    let mut stderr = io::stderr().lock();
    for _ in 0..(FINITE_FAKE_TOOL_OUTPUT_BYTES / chunk.len()) {
        stderr
            .write_all(&chunk)
            .expect("large-output fixture should write stderr");
    }
    stderr
        .flush()
        .expect("large-output fixture should flush stderr");
}

#[test]
fn media_match_parked_process_fixture() {
    if !invoked_as_exact_fixture(PARKED_FIXTURE_TEST) {
        return;
    }
    loop {
        std::thread::park();
    }
}

#[test]
fn version_probe_selects_first_nonempty_line_and_accepts_unterminated_final_line() {
    assert_eq!(
        shell_probe(COMPLETE_VERSION_SCRIPT).expect("complete version output should parse"),
        "ffprobe version 8.0"
    );
    assert_eq!(
        shell_probe(UNTERMINATED_VERSION_SCRIPT)
            .expect("an unterminated final version line should parse"),
        "ffprobe version 8.0-no-newline"
    );
}

#[test]
fn version_probe_preserves_nonzero_exit_status() {
    let error = shell_probe(NONZERO_EXIT_SCRIPT).expect_err("nonzero fake tool must be rejected");
    assert_eq!(error, "exited with status 23");
}

#[test]
fn version_probe_rejects_unusable_success_output() {
    let accepted = [
        ("empty", shell_probe(EMPTY_SUCCESS_SCRIPT)),
        ("invalid-utf8", shell_probe(INVALID_UTF8_SCRIPT)),
        ("unrelated", shell_probe(UNRELATED_SUCCESS_SCRIPT)),
    ]
    .into_iter()
    .filter_map(|(case, result)| result.ok().map(|version| (case, version)))
    .collect::<Vec<_>>();

    assert!(
        accepted.is_empty(),
        "successful process without a valid tool version must be rejected: {accepted:?}"
    );
}

#[test]
fn timed_out_version_probe_reaps_process_and_releases_executable() {
    let fixture = FakeMediaMatchTool::new("timeout-cleanup");
    let args = fixture_args(PARKED_FIXTURE_TEST);
    let started = Instant::now();
    let error =
        probe_executable_output_with_timeout(fixture.executable(), &args, PROCESS_FIXTURE_TIMEOUT)
            .expect_err("parked fake tool should time out");

    assert!(error.contains("timed out"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "bounded timeout must not turn into an unbounded process wait"
    );
    fixture.assert_process_and_image_released();
}

#[test]
fn version_probe_drains_finite_output_larger_than_pipe_capacity() {
    let fixture = FakeMediaMatchTool::new("finite-large-output");
    let args = fixture_args(LARGE_STDOUT_FIXTURE_TEST);
    let output =
        probe_executable_output_with_timeout(fixture.executable(), &args, PROCESS_FIXTURE_TIMEOUT)
            .expect("finite fake-tool output must drain without any process or pipe error");
    fixture.assert_process_and_image_released();
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), MEDIA_MATCH_VERSION_CAPTURE_LIMIT_BYTES);
    assert_eq!(output.stderr.len(), MEDIA_MATCH_VERSION_CAPTURE_LIMIT_BYTES);
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
}
