use super::*;

#[test]
fn automatic_install_is_rejected_without_creating_a_stage() {
    let root = tempfile::tempdir().unwrap();
    let mut reported_progress = false;
    let error = install_or_update_managed_stream_helper_with_progress(root.path(), None, |_| {
        reported_progress = true;
    })
    .unwrap_err();
    assert!(error.contains("only implemented for Windows"));
    assert!(!reported_progress);
    assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
}
