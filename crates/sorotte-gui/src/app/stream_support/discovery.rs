use std::path::{Path, PathBuf};

use super::paths::{
    discover_managed_stream_helper_component, managed_downloader_file_name,
    managed_js_runtime_file_name,
};
use super::process::{find_executable_on_path, probe_executable_version};
use super::{
    ManagedStreamHelperComponent, StreamHelperAttachMode, StreamHelperComponentProbe,
    StreamHelperDiscovery, StreamHelperSource,
};

pub(in crate::app::stream_support) fn probe_stream_helper_component(
    component: ManagedStreamHelperComponent,
    attach_mode: StreamHelperAttachMode,
    managed: Option<PathBuf>,
    environment: Option<PathBuf>,
) -> StreamHelperComponentProbe {
    let describe_effective_path =
        |path: PathBuf, source: StreamHelperSource, source_label: &'static str| {
            match probe_executable_version(&path, &["--version"]) {
                Ok(version) => StreamHelperComponentProbe {
                    effective_path: Some(path.clone()),
                    effective_source: Some(source),
                    effective_version: Some(version.clone()),
                    effective_error: None,
                    status: format!("{source_label}: {version} ({})", path.display()),
                },
                Err(error) => StreamHelperComponentProbe {
                    effective_path: Some(path.clone()),
                    effective_source: Some(source),
                    effective_version: None,
                    effective_error: Some(error.clone()),
                    status: format!(
                        "{source_label} at '{}' is unusable: {error}",
                        path.display()
                    ),
                },
            }
        };

    match attach_mode {
        StreamHelperAttachMode::ManagedPlayer => managed
            .map(|path| {
                describe_effective_path(path, StreamHelperSource::Managed, "Managed install")
            })
            .or_else(|| {
                environment.map(|path| {
                    describe_effective_path(path, StreamHelperSource::Environment, "PATH")
                })
            })
            .unwrap_or_else(|| StreamHelperComponentProbe {
                effective_path: None,
                effective_source: None,
                effective_version: None,
                effective_error: None,
                status: format!(
                    "Missing from Sorotte's managed install and PATH for {}.",
                    component.display_name()
                ),
            }),
        StreamHelperAttachMode::ExternalPlayer => environment
            .map(|path| describe_effective_path(path, StreamHelperSource::Environment, "PATH"))
            .unwrap_or_else(|| {
                if let Some(path) = managed {
                    return StreamHelperComponentProbe {
                        effective_path: None,
                        effective_source: None,
                        effective_version: None,
                        effective_error: None,
                        status: format!(
                            "Managed install present at '{}', but an external mpv process can only use PATH-visible helpers.",
                            path.display()
                        ),
                    };
                }
                StreamHelperComponentProbe {
                    effective_path: None,
                    effective_source: None,
                    effective_version: None,
                    effective_error: None,
                    status: format!(
                        "Missing from PATH for the external player: {}.",
                        component.display_name()
                    ),
                }
            }),
    }
}

pub(in crate::app::stream_support) fn discover_stream_helpers(
    root: Option<&Path>,
) -> StreamHelperDiscovery {
    StreamHelperDiscovery {
        managed_downloader: root.and_then(|root| {
            discover_managed_stream_helper_component(root, managed_downloader_file_name())
        }),
        environment_downloader: find_executable_on_path(&[
            managed_downloader_file_name(),
            "yt-dlp",
            "youtube-dl.exe",
            "youtube-dl",
        ]),
        managed_js_runtime: root.and_then(|root| {
            discover_managed_stream_helper_component(root, managed_js_runtime_file_name())
        }),
        environment_js_runtime: find_executable_on_path(&[managed_js_runtime_file_name(), "deno"]),
    }
}
