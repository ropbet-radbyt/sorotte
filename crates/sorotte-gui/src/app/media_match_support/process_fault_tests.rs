use super::{MediaMatchTool, probe_executable_version};
use std::path::Path;

#[cfg(windows)]
fn shell_probe(script: &str) -> Result<String, String> {
    probe_executable_version(
        Path::new("cmd.exe"),
        &["/d", "/s", "/c", script],
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
    "(echo.&echo ffprobe version 8.0&echo configuration details)&exit /b 0";
#[cfg(not(windows))]
const COMPLETE_VERSION_SCRIPT: &str = "printf '\\nffprobe version 8.0\\nconfiguration details\\n'";

#[cfg(windows)]
const UNTERMINATED_VERSION_SCRIPT: &str = "<nul set /p =ffprobe version 8.0-no-newline&exit /b 0";
#[cfg(not(windows))]
const UNTERMINATED_VERSION_SCRIPT: &str = "printf 'ffprobe version 8.0-no-newline'";

#[cfg(windows)]
const NONZERO_EXIT_SCRIPT: &str = ">&2 echo probe failure&exit /b 23";
#[cfg(not(windows))]
const NONZERO_EXIT_SCRIPT: &str = "printf 'probe failure' >&2; exit 23";

#[cfg(windows)]
const EMPTY_SUCCESS_SCRIPT: &str = "exit 0";
#[cfg(not(windows))]
const EMPTY_SUCCESS_SCRIPT: &str = "exit 0";

#[cfg(windows)]
const UNRELATED_SUCCESS_SCRIPT: &str = "<nul set /p =not a media tool&exit /b 0";
#[cfg(not(windows))]
const UNRELATED_SUCCESS_SCRIPT: &str = "printf 'not a media tool'";

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
    assert!(
        error.contains("23") && error.contains("probe failure"),
        "{error}"
    );
}

#[test]
fn version_probe_rejects_unusable_success_output() {
    let accepted = [
        ("empty", shell_probe(EMPTY_SUCCESS_SCRIPT)),
        ("unrelated", shell_probe(UNRELATED_SUCCESS_SCRIPT)),
    ]
    .into_iter()
    .filter_map(|(case, result)| result.ok().map(|version| (case, version)))
    .collect::<Vec<_>>();

    assert!(
        accepted.is_empty(),
        "successful process without a valid tool version must be rejected: {accepted:?}"
    );
    assert_eq!(
        crate::app::helper_tools::HelperTool::Ffprobe
            .parse_version(b"ff\xffprobe version 8.0\n")
            .expect_err("invalid UTF-8 must be rejected before banner matching"),
        "version banner was not valid UTF-8"
    );
}
