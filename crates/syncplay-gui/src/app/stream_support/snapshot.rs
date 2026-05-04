use std::path::Path;

use crate::app::shell_state::{
    GuiStreamHelperHealth, GuiStreamHelperRuntimeSnapshot, GuiStreamTargetKind,
    browser_stream_target_kind,
};

use super::discovery::{discover_stream_helpers, probe_stream_helper_component};
use super::metadata::{load_managed_stream_helper_metadata, managed_installation_is_stale};
use super::paths::{discovered_managed_stream_helper_bin_dir, managed_stream_helper_bin_dir};
use super::{
    ManagedStreamHelperComponent, StreamHelperAttachMode, StreamHelperExecutable,
    StreamHelperRuntimeSnapshotDetails, StreamHelperSource,
};

pub(in crate::app) fn probe_stream_helper_runtime_snapshot(
    root: Option<&Path>,
    attach_mode: StreamHelperAttachMode,
    target: Option<&str>,
) -> GuiStreamHelperRuntimeSnapshot {
    let extractor_target = target.and_then(|target| {
        (browser_stream_target_kind(target, None) == GuiStreamTargetKind::ExtractorPageUrl)
            .then_some(target)
    });
    let install_supported = cfg!(windows) && root.is_some();
    let integration_supported = root.is_some();
    let discovery = discover_stream_helpers(root);
    let metadata = root.and_then(load_managed_stream_helper_metadata);
    let install_location = root.map(|root| {
        discovered_managed_stream_helper_bin_dir(root)
            .unwrap_or_else(|| managed_stream_helper_bin_dir(root))
            .display()
            .to_string()
    });
    let Some(target) = extractor_target else {
        let snapshot_details = StreamHelperRuntimeSnapshotDetails {
            install_location: install_location.clone(),
            downloader_status: Some(startup_component_status(
                ManagedStreamHelperComponent::Downloader,
                attach_mode,
                discovery.managed_downloader,
                discovery.environment_downloader,
                metadata
                    .as_ref()
                    .and_then(|metadata| metadata.downloader_version.as_deref()),
            )),
            js_runtime_status: Some(startup_component_status(
                ManagedStreamHelperComponent::JsRuntime,
                attach_mode,
                discovery.managed_js_runtime,
                discovery.environment_js_runtime,
                metadata
                    .as_ref()
                    .and_then(|metadata| metadata.js_runtime_version.as_deref()),
            )),
            open_install_location_available: root.is_some(),
        };
        return runtime_snapshot_with_details(
            GuiStreamHelperHealth::Healthy,
            None,
            None,
            install_supported,
            integration_supported,
            false,
            snapshot_details,
        );
    };
    let downloader_probe = probe_stream_helper_component(
        ManagedStreamHelperComponent::Downloader,
        attach_mode,
        discovery.managed_downloader.clone(),
        discovery.environment_downloader.clone(),
    );
    let js_runtime_probe = probe_stream_helper_component(
        ManagedStreamHelperComponent::JsRuntime,
        attach_mode,
        discovery.managed_js_runtime.clone(),
        discovery.environment_js_runtime.clone(),
    );
    let snapshot_details = StreamHelperRuntimeSnapshotDetails {
        install_location: install_location.clone(),
        downloader_status: Some(downloader_probe.status.clone()),
        js_runtime_status: Some(js_runtime_probe.status.clone()),
        open_install_location_available: root.is_some(),
    };
    let status_snapshot = |health: GuiStreamHelperHealth,
                           message: Option<String>,
                           target: Option<&str>,
                           retry_available: bool| {
        runtime_snapshot_with_details(
            health,
            message,
            target,
            install_supported,
            integration_supported,
            retry_available,
            snapshot_details.clone(),
        )
    };

    if attach_mode == StreamHelperAttachMode::ExternalPlayer
        && (downloader_probe.effective_path.is_none() || js_runtime_probe.effective_path.is_none())
    {
        return status_snapshot(
            GuiStreamHelperHealth::ExternalPlayerUnmanaged,
            Some(
                "This URL needs yt-dlp and Deno to be visible to the already-running external mpv process. Install them globally or relaunch mpv from Syncplay after setup."
                    .to_owned(),
            ),
            Some(target),
            true,
        );
    }

    let Some(downloader_path) = downloader_probe.effective_path.clone() else {
        let health = if install_supported {
            GuiStreamHelperHealth::MissingDownloader
        } else {
            GuiStreamHelperHealth::UnsupportedPlatform
        };
        let message = if install_supported {
            "Extractor-backed page URLs need yt-dlp before mpv can load them. Import it or install the managed helper."
                .to_owned()
        } else {
            "Automatic helper installation is not available on this platform yet. Import yt-dlp and Deno or install them manually."
                .to_owned()
        };
        return status_snapshot(health, Some(message), Some(target), true);
    };
    if let Some(error) = downloader_probe.effective_error.clone() {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some(format!("yt-dlp could not be executed: {error}")),
            Some(target),
            true,
        );
    }

    let Some(js_runtime_path) = js_runtime_probe.effective_path.clone() else {
        let health = if install_supported {
            GuiStreamHelperHealth::MissingJsRuntime
        } else {
            GuiStreamHelperHealth::UnsupportedPlatform
        };
        let message = if install_supported {
            "This URL needs a JavaScript runtime for yt-dlp extraction. Import Deno or install the managed runtime."
                .to_owned()
        } else {
            "Automatic helper installation is not available on this platform yet. Import yt-dlp and Deno or install them manually."
                .to_owned()
        };
        return status_snapshot(health, Some(message), Some(target), true);
    };
    if let Some(error) = js_runtime_probe.effective_error.clone() {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some(format!("Deno could not be executed: {error}")),
            Some(target),
            true,
        );
    }
    let Some(downloader_source) = downloader_probe.effective_source else {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some("yt-dlp discovery reported a path without a source.".to_owned()),
            Some(target),
            true,
        );
    };
    let downloader = StreamHelperExecutable {
        path: downloader_path,
        source: downloader_source,
        version: downloader_probe.effective_version.clone(),
    };
    let Some(js_runtime_source) = js_runtime_probe.effective_source else {
        return status_snapshot(
            GuiStreamHelperHealth::Broken,
            Some("Deno discovery reported a path without a source.".to_owned()),
            Some(target),
            true,
        );
    };
    let js_runtime = StreamHelperExecutable {
        path: js_runtime_path,
        source: js_runtime_source,
        version: js_runtime_probe.effective_version.clone(),
    };

    let using_managed_installation = downloader.source == StreamHelperSource::Managed
        || js_runtime.source == StreamHelperSource::Managed;
    if using_managed_installation && managed_installation_is_stale(metadata.as_ref()) {
        return status_snapshot(
            GuiStreamHelperHealth::Stale,
            Some(format!(
                "Managed stream helper found at '{}' and '{}', but it should be refreshed before retrying this URL.",
                downloader.path.display(),
                js_runtime.path.display()
            )),
            Some(target),
            true,
        );
    }

    status_snapshot(GuiStreamHelperHealth::Healthy, None, Some(target), true)
}

pub(in crate::app) fn probe_stream_helper_startup_snapshot(
    root: Option<&Path>,
    attach_mode: StreamHelperAttachMode,
) -> GuiStreamHelperRuntimeSnapshot {
    let install_supported = cfg!(windows) && root.is_some();
    let integration_supported = root.is_some();
    let discovery = discover_stream_helpers(root);
    let metadata = root.and_then(load_managed_stream_helper_metadata);
    let install_location = root.map(|root| {
        discovered_managed_stream_helper_bin_dir(root)
            .unwrap_or_else(|| managed_stream_helper_bin_dir(root))
            .display()
            .to_string()
    });
    let details = StreamHelperRuntimeSnapshotDetails {
        install_location,
        downloader_status: Some(startup_component_status(
            ManagedStreamHelperComponent::Downloader,
            attach_mode,
            discovery.managed_downloader,
            discovery.environment_downloader,
            metadata
                .as_ref()
                .and_then(|metadata| metadata.downloader_version.as_deref()),
        )),
        js_runtime_status: Some(startup_component_status(
            ManagedStreamHelperComponent::JsRuntime,
            attach_mode,
            discovery.managed_js_runtime,
            discovery.environment_js_runtime,
            metadata
                .as_ref()
                .and_then(|metadata| metadata.js_runtime_version.as_deref()),
        )),
        open_install_location_available: root.is_some(),
    };
    runtime_snapshot_with_details(
        GuiStreamHelperHealth::Healthy,
        None,
        None,
        install_supported,
        integration_supported,
        false,
        details,
    )
}

fn startup_component_status(
    component: ManagedStreamHelperComponent,
    attach_mode: StreamHelperAttachMode,
    managed: Option<std::path::PathBuf>,
    environment: Option<std::path::PathBuf>,
    managed_version: Option<&str>,
) -> String {
    match attach_mode {
        StreamHelperAttachMode::ManagedPlayer => managed
            .map(|path| startup_present_status("Managed install", path, managed_version))
            .or_else(|| environment.map(|path| startup_present_status("PATH", path, None)))
            .unwrap_or_else(|| {
                format!(
                    "Missing from Syncplay's managed install and PATH for {}.",
                    component.display_name()
                )
            }),
        StreamHelperAttachMode::ExternalPlayer => environment
            .map(|path| startup_present_status("PATH", path, None))
            .unwrap_or_else(|| {
                if let Some(path) = managed {
                    return format!(
                        "Managed install present at '{}', but an external mpv process can only use PATH-visible helpers.",
                        path.display()
                    );
                }
                format!(
                    "Missing from PATH for the external player: {}.",
                    component.display_name()
                )
            }),
    }
}

fn startup_present_status(
    source_label: &str,
    path: std::path::PathBuf,
    version: Option<&str>,
) -> String {
    match version {
        Some(version) => format!("{source_label}: {version} ({})", path.display()),
        None => format!(
            "{source_label} present at '{}'; version check pending.",
            path.display()
        ),
    }
}

fn runtime_snapshot_with_details(
    health: GuiStreamHelperHealth,
    message: Option<String>,
    target: Option<&str>,
    install_supported: bool,
    integration_supported: bool,
    retry_available: bool,
    details: StreamHelperRuntimeSnapshotDetails,
) -> GuiStreamHelperRuntimeSnapshot {
    GuiStreamHelperRuntimeSnapshot {
        health,
        message,
        target: target.map(str::to_owned),
        install_supported,
        integration_supported,
        retry_available,
        install_location: details.install_location,
        downloader_status: details.downloader_status,
        js_runtime_status: details.js_runtime_status,
        open_install_location_available: details.open_install_location_available,
    }
}
