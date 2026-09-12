use super::{HelperTool, tests::tool_fixture};
use std::{fs, sync::atomic::AtomicBool, time::Duration};

#[test]
fn probes_validate_each_tool_and_preserve_exit_failure() {
    let root = tempfile::tempdir().unwrap();
    for (tool, mode) in [
        (HelperTool::YtDlp, "yt-dlp"),
        (HelperTool::Deno, "deno"),
        (HelperTool::Ffmpeg, "ffmpeg"),
        (HelperTool::Ffprobe, "ffprobe"),
    ] {
        let result = tool.probe(&tool_fixture(root.path(), mode), None);
        assert!(result.is_ok(), "{tool:?}: {result:?}");
        let error = tool
            .probe(&tool_fixture(root.path(), "wrong"), None)
            .unwrap_err();
        assert!(error.contains("version banner"), "{tool:?}: {error}");
    }
    let error = HelperTool::YtDlp
        .probe(&tool_fixture(root.path(), "fail"), None)
        .unwrap_err();
    assert!(
        error.contains("17") && error.contains("fixture failure"),
        "{error}"
    );
    assert!(
        HelperTool::Ffprobe
            .parse_version(b"ff\xffprobe version 8.0")
            .is_err()
    );
}

#[test]
fn bounded_probe_rejects_flood_and_reaps_cancelled_tool() {
    let root = tempfile::tempdir().unwrap();
    let flood = tool_fixture(root.path(), "flood");
    assert!(HelperTool::YtDlp.probe(&flood, None).is_err());
    fs::remove_file(flood).unwrap();
    let hang = tool_fixture(root.path(), "hang");
    let error = sorotte_media_match::run_tool_probe(
        "fixture",
        &hang,
        &[],
        Duration::from_millis(150),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    fs::remove_file(hang).unwrap();
    let cancelled = AtomicBool::new(true);
    assert!(
        HelperTool::Deno
            .probe(&tool_fixture(root.path(), "deno"), Some(&cancelled))
            .is_err()
    );
}
