use crate::app::shell_state::{
    GuiStreamHelperHealth, GuiStreamTargetKind, browser_stream_target_kind,
};

use super::install::{
    import_managed_stream_helper_downloader, import_managed_stream_helper_js_runtime,
    validate_installed_stream_helper_component,
};
use super::metadata::{
    current_unix_seconds, load_managed_stream_helper_metadata, managed_installation_is_stale,
};
use super::paths::{managed_stream_helper_bin_dir, managed_stream_helper_path_prefixes};
use super::process::find_executable_on_path;
use super::snapshot::{probe_stream_helper_runtime_snapshot, probe_stream_helper_startup_snapshot};
use super::{
    ManagedStreamHelperComponent, ManagedStreamHelperMetadata, STREAM_HELPER_STALE_AFTER,
    StreamHelperAttachMode,
};

fn version_capable_executable() -> std::path::PathBuf {
    [
        "python.exe",
        "python",
        "python3.exe",
        "python3",
        "pwsh.exe",
        "pwsh",
        "powershell.exe",
        "powershell",
        "node.exe",
        "node",
    ]
    .iter()
    .find_map(|candidate| find_executable_on_path(&[*candidate]))
    .expect("a version-capable executable should be available on PATH for stream-helper tests")
}

#[test]
fn managed_stream_helper_path_prefixes_include_existing_bin_dir() {
    let root = std::env::temp_dir().join(format!(
        "syncplay-stream-helper-test-{}",
        std::process::id()
    ));
    let bin_dir = managed_stream_helper_bin_dir(&root);
    std::fs::create_dir_all(&bin_dir).expect("managed helper bin dir should be created");

    assert_eq!(
        managed_stream_helper_path_prefixes(Some(root.as_path())),
        vec![bin_dir.clone()]
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn probe_stream_helper_runtime_snapshot_ignores_direct_media_urls() {
    let snapshot = probe_stream_helper_runtime_snapshot(
        None,
        StreamHelperAttachMode::ManagedPlayer,
        Some("https://cdn.example.com/video.m3u8"),
    );

    assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
    assert_eq!(
        browser_stream_target_kind("https://cdn.example.com/video.m3u8", None,),
        GuiStreamTargetKind::DirectMediaUrl
    );
    assert_eq!(snapshot.message, None);
    assert_eq!(snapshot.target, None);
}

#[test]
fn probe_stream_helper_startup_snapshot_does_not_execute_helper_binaries() {
    let root = std::env::temp_dir().join(format!(
        "syncplay-stream-helper-startup-snapshot-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let bin_dir = managed_stream_helper_bin_dir(&root);
    std::fs::create_dir_all(&bin_dir).expect("managed helper bin dir should be created");
    std::fs::write(
        bin_dir.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        }),
        b"not an executable",
    )
    .expect("fake downloader should be written");
    std::fs::write(
        bin_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" }),
        b"not an executable",
    )
    .expect("fake runtime should be written");

    let snapshot =
        probe_stream_helper_startup_snapshot(Some(&root), StreamHelperAttachMode::ManagedPlayer);

    assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
    assert!(
        snapshot
            .downloader_status
            .as_deref()
            .is_some_and(|status| status.contains("version check pending")),
        "startup snapshot should use metadata/discovery only, got {:?}",
        snapshot.downloader_status
    );
    assert!(
        snapshot
            .js_runtime_status
            .as_deref()
            .is_some_and(|status| status.contains("version check pending")),
        "startup snapshot should use metadata/discovery only, got {:?}",
        snapshot.js_runtime_status
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn probe_stream_helper_runtime_snapshot_without_extractor_target_does_not_execute_helper_binaries()
{
    let root = std::env::temp_dir().join(format!(
        "syncplay-stream-helper-runtime-no-target-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let bin_dir = managed_stream_helper_bin_dir(&root);
    std::fs::create_dir_all(&bin_dir).expect("managed helper bin dir should be created");
    std::fs::write(
        bin_dir.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        }),
        b"not an executable",
    )
    .expect("fake downloader should be written");
    std::fs::write(
        bin_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" }),
        b"not an executable",
    )
    .expect("fake runtime should be written");

    let snapshot = probe_stream_helper_runtime_snapshot(
        Some(&root),
        StreamHelperAttachMode::ManagedPlayer,
        None,
    );

    assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
    assert!(
        snapshot
            .downloader_status
            .as_deref()
            .is_some_and(|status| status.contains("version check pending")),
        "runtime snapshot without a target should use metadata/discovery only, got {:?}",
        snapshot.downloader_status
    );
    assert!(
        snapshot
            .js_runtime_status
            .as_deref()
            .is_some_and(|status| status.contains("version check pending")),
        "runtime snapshot without a target should use metadata/discovery only, got {:?}",
        snapshot.js_runtime_status
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn managed_installation_staleness_depends_on_metadata_age() {
    assert!(managed_installation_is_stale(None));
    assert!(managed_installation_is_stale(Some(
        &ManagedStreamHelperMetadata::default()
    )));

    let fresh_metadata = ManagedStreamHelperMetadata {
        installed_at_unix_seconds: Some(current_unix_seconds()),
        downloader_version: Some("test".to_owned()),
        js_runtime_version: Some("test".to_owned()),
    };
    assert!(!managed_installation_is_stale(Some(&fresh_metadata)));

    let stale_metadata = ManagedStreamHelperMetadata {
        installed_at_unix_seconds: Some(
            current_unix_seconds().saturating_sub(STREAM_HELPER_STALE_AFTER.as_secs() + 1),
        ),
        downloader_version: Some("test".to_owned()),
        js_runtime_version: Some("test".to_owned()),
    };
    assert!(managed_installation_is_stale(Some(&stale_metadata)));
}

#[test]
fn importing_stream_helper_binaries_populates_managed_helper_paths_and_metadata() {
    let root = std::env::temp_dir().join(format!(
        "syncplay-stream-helper-import-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let source_executable = version_capable_executable();

    let downloader_message = import_managed_stream_helper_downloader(&root, &source_executable)
        .expect("downloader import should succeed");
    assert!(downloader_message.contains("Imported yt-dlp"));
    assert!(
        managed_stream_helper_bin_dir(&root)
            .join(if cfg!(windows) {
                "yt-dlp.exe"
            } else {
                "yt-dlp"
            })
            .is_file()
    );

    let js_runtime_message = import_managed_stream_helper_js_runtime(&root, &source_executable)
        .expect("js-runtime import should succeed");
    assert!(js_runtime_message.contains("Imported Deno"));
    assert!(
        managed_stream_helper_bin_dir(&root)
            .join(if cfg!(windows) { "deno.exe" } else { "deno" })
            .is_file()
    );

    let metadata = load_managed_stream_helper_metadata(&root)
        .expect("managed helper metadata should be written");
    assert!(metadata.downloader_version.is_some());
    assert!(metadata.js_runtime_version.is_some());

    let snapshot = probe_stream_helper_runtime_snapshot(
        Some(root.as_path()),
        StreamHelperAttachMode::ManagedPlayer,
        Some("https://www.youtube.com/watch?v=UyjIPZfygTk"),
    );
    assert_eq!(snapshot.health, GuiStreamHelperHealth::Healthy);
    assert!(snapshot.integration_supported);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn installed_stream_helper_validation_rejects_unusable_binaries() {
    let root = std::env::temp_dir().join(format!(
        "syncplay-stream-helper-invalid-install-{}-{}",
        std::process::id(),
        current_unix_seconds()
    ));
    let path = managed_stream_helper_bin_dir(&root).join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    std::fs::create_dir_all(path.parent().expect("managed helper dir should exist"))
        .expect("managed helper dir should be created");
    std::fs::write(&path, b"not an executable").expect("invalid helper payload should be written");

    let error =
        validate_installed_stream_helper_component(&path, ManagedStreamHelperComponent::Downloader)
            .expect_err("invalid helper payload should fail validation");
    assert!(error.contains("yt-dlp could not be executed after install"));
    assert!(
        !path.exists(),
        "failed install validation should remove the unusable helper payload"
    );

    let _ = std::fs::remove_dir_all(root);
}
