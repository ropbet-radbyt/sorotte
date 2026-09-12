use super::{MediaMatchTool, probe_executable_version};
use crate::app::helper_tools::tests::tool_fixture;

fn fixture_probe(mode: &str) -> Result<String, String> {
    let root = tempfile::tempdir().unwrap();
    probe_executable_version(
        &tool_fixture(root.path(), mode),
        &["-version"],
        MediaMatchTool::Ffprobe,
    )
}

#[test]
fn version_probe_selects_first_nonempty_line_and_accepts_unterminated_final_line() {
    assert_eq!(
        fixture_probe("probe-full").expect("complete version output should parse"),
        "ffprobe version 8.0"
    );
    assert_eq!(
        fixture_probe("probe-unterminated")
            .expect("an unterminated final version line should parse"),
        "ffprobe version 8.0-no-newline"
    );
}

#[test]
fn version_probe_preserves_nonzero_exit_status() {
    let error = fixture_probe("probe-fail").expect_err("nonzero fake tool must be rejected");
    assert!(
        error.contains("23") && error.contains("probe failure"),
        "{error}"
    );
}

#[test]
fn version_probe_rejects_unusable_success_output() {
    let accepted = [
        ("empty", fixture_probe("empty")),
        ("unrelated", fixture_probe("wrong")),
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
