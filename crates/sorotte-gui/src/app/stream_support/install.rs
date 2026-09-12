use super::metadata::{current_unix_seconds, load_managed_stream_helper_metadata};
use super::paths::{
    managed_downloader_file_name, managed_js_runtime_file_name, managed_stream_helper_bin_dir,
};
use super::{
    ManagedStreamHelperComponent, ManagedStreamHelperMetadata, STREAM_HELPER_DOWNLOAD_TIMEOUT,
    StreamHelperRemediationProgress, YTDLP_WINDOWS_LATEST_URL,
};
use crate::app::helper_tools::{ToolInstall, download_to_path, extract_executable};
use std::{env, path::Path, sync::atomic::AtomicBool};

#[cfg(test)]
#[path = "install/tests.rs"]
mod tests;

pub(in crate::app) fn install_or_update_managed_stream_helper_with_progress(
    root: &Path,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    if !cfg!(windows) {
        return Err(
            "Automatic stream-helper installation is only implemented for Windows in this release."
                .to_owned(),
        );
    }
    install_managed_stream_helper_from_sources(
        root,
        YTDLP_WINDOWS_LATEST_URL,
        &windows_deno_latest_url()?,
        cancel,
        progress,
    )
}

fn install_managed_stream_helper_from_sources(
    root: &Path,
    downloader_url: &str,
    runtime_url: &str,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    let bin = managed_stream_helper_bin_dir(root);
    let install = ToolInstall::begin(&bin, cancel)?;
    let downloader = install.path(managed_downloader_file_name());
    let runtime = install.path(managed_js_runtime_file_name());
    progress(StreamHelperRemediationProgress::new(
        "Downloading yt-dlp",
        None,
        0.1,
    ));
    download_to_path(
        downloader_url,
        &downloader,
        STREAM_HELPER_DOWNLOAD_TIMEOUT,
        cancel,
        |done, total| {
            progress(StreamHelperRemediationProgress::new(
                "Downloading yt-dlp",
                Some(format!("{done} bytes")),
                total.map_or(0.1, |size| 0.1 + 0.3 * done as f32 / size.max(1) as f32),
            ));
        },
    )?;
    let archive = install.path("deno.zip");
    progress(StreamHelperRemediationProgress::new(
        "Downloading Deno",
        None,
        0.4,
    ));
    download_to_path(
        runtime_url,
        &archive,
        STREAM_HELPER_DOWNLOAD_TIMEOUT,
        cancel,
        |done, total| {
            progress(StreamHelperRemediationProgress::new(
                "Downloading Deno",
                Some(format!("{done} bytes")),
                total.map_or(0.4, |size| 0.4 + 0.3 * done as f32 / size.max(1) as f32),
            ));
        },
    )?;
    extract_executable(&archive, "deno.exe", &runtime, cancel)?;
    progress(StreamHelperRemediationProgress::new(
        "Validating stream helper binaries",
        None,
        0.75,
    ));
    let downloader_version = ManagedStreamHelperComponent::Downloader
        .helper_tool()
        .probe(&downloader, cancel)?;
    let js_runtime_version = ManagedStreamHelperComponent::JsRuntime
        .helper_tool()
        .probe(&runtime, cancel)?;
    let metadata = ManagedStreamHelperMetadata {
        installed_at_unix_seconds: Some(current_unix_seconds()),
        downloader_version: Some(downloader_version),
        js_runtime_version: Some(js_runtime_version),
    };
    install.write_metadata(&metadata)?;
    install.commit(
        &[
            managed_downloader_file_name(),
            managed_js_runtime_file_name(),
        ],
        cancel,
    )?;
    Ok(format!(
        "Installed managed stream helper into '{}'.",
        bin.display()
    ))
}

pub(in crate::app) fn import_managed_stream_helper_downloader_with_progress(
    root: &Path,
    source: &Path,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    import_managed_stream_helper_component(
        root,
        source,
        ManagedStreamHelperComponent::Downloader,
        cancel,
        progress,
    )
}

pub(in crate::app) fn import_managed_stream_helper_js_runtime_with_progress(
    root: &Path,
    source: &Path,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    import_managed_stream_helper_component(
        root,
        source,
        ManagedStreamHelperComponent::JsRuntime,
        cancel,
        progress,
    )
}

fn import_managed_stream_helper_component(
    root: &Path,
    source: &Path,
    component: ManagedStreamHelperComponent,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    let bin = managed_stream_helper_bin_dir(root);
    let install = ToolInstall::begin(&bin, cancel)?;
    progress(StreamHelperRemediationProgress::new(
        format!("Importing {}", component.display_name()),
        Some(source.display().to_string()),
        0.15,
    ));
    let staged = install.copy_executable(source, component.target_file_name(), cancel)?;
    progress(StreamHelperRemediationProgress::new(
        format!("Validating {}", component.display_name()),
        None,
        0.6,
    ));
    let version = component.helper_tool().probe(&staged, cancel)?;
    let mut metadata = load_managed_stream_helper_metadata(root).unwrap_or_default();
    metadata.installed_at_unix_seconds = Some(current_unix_seconds());
    component.assign_version(&mut metadata, version);
    install.write_metadata(&metadata)?;
    install.commit(&[component.target_file_name()], cancel)?;
    Ok(format!(
        "Imported {} into '{}'.",
        component.display_name(),
        bin.join(component.target_file_name()).display()
    ))
}

#[cfg(test)]
pub(in crate::app::stream_support) fn import_managed_stream_helper_downloader(
    root: &Path,
    source: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_downloader_with_progress(root, source, None, |_| {})
}

#[cfg(test)]
pub(in crate::app::stream_support) fn import_managed_stream_helper_js_runtime(
    root: &Path,
    source: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_js_runtime_with_progress(root, source, None, |_| {})
}

fn windows_deno_latest_url() -> Result<String, String> {
    let asset = match env::consts::ARCH {
        "x86_64" => "deno-x86_64-pc-windows-msvc.zip",
        "aarch64" => "deno-aarch64-pc-windows-msvc.zip",
        other => {
            return Err(format!(
                "automatic Deno installation is unsupported on Windows architecture '{other}'"
            ));
        }
    };
    Ok(format!(
        "https://github.com/denoland/deno/releases/latest/download/{asset}"
    ))
}
