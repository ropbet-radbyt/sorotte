use std::{
    env, fs,
    io::{Cursor, Read},
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use zip::ZipArchive;

use super::metadata::{
    current_unix_seconds, load_managed_stream_helper_metadata, save_managed_stream_helper_metadata,
};
use super::paths::{
    managed_downloader_file_name, managed_js_runtime_file_name, managed_stream_helper_bin_dir,
};
use super::process::{
    download_bytes, download_to_path, helper_http_client, probe_executable_version,
};
use super::{
    ManagedStreamHelperComponent, ManagedStreamHelperMetadata, StreamHelperRemediationProgress,
    YTDLP_WINDOWS_LATEST_URL,
};

pub(in crate::app) fn install_or_update_managed_stream_helper_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    if !cfg!(windows) {
        return Err(
            "Automatic stream-helper installation is only implemented for Windows in this release."
                .to_owned(),
        );
    }

    let bin_dir = managed_stream_helper_bin_dir(root);
    progress(StreamHelperRemediationProgress::new(
        "Preparing stream helper remediation",
        Some(format!(
            "Creating managed stream helper directory at '{}'.",
            bin_dir.display()
        )),
        0.08,
    ));
    fs::create_dir_all(&bin_dir).map_err(|error| {
        format!(
            "failed to create managed stream helper directory '{}': {error}",
            bin_dir.display()
        )
    })?;

    let client = helper_http_client()?;
    let yt_dlp_path = bin_dir.join(managed_downloader_file_name());
    let deno_path = bin_dir.join(managed_js_runtime_file_name());
    progress(StreamHelperRemediationProgress::new(
        "Downloading yt-dlp",
        Some(format!("Saving yt-dlp into '{}'.", yt_dlp_path.display())),
        0.25,
    ));
    download_to_path(&client, YTDLP_WINDOWS_LATEST_URL, &yt_dlp_path)?;
    progress(StreamHelperRemediationProgress::new(
        "Downloading Deno",
        Some(format!("Saving Deno into '{}'.", deno_path.display())),
        0.50,
    ));
    let deno_bytes = download_bytes(&client, &windows_deno_latest_url()?)?;
    extract_deno_executable_from_zip(&deno_bytes, &deno_path)?;

    progress(StreamHelperRemediationProgress::new(
        "Validating stream helper binaries",
        Some("Checking that yt-dlp and Deno can be executed.".to_owned()),
        0.72,
    ));
    let downloader_version = validate_installed_stream_helper_component(
        &yt_dlp_path,
        ManagedStreamHelperComponent::Downloader,
    )?;
    let js_runtime_version = validate_installed_stream_helper_component(
        &deno_path,
        ManagedStreamHelperComponent::JsRuntime,
    )?;
    progress(StreamHelperRemediationProgress::new(
        "Saving stream helper metadata",
        Some("Recording the installed helper versions for later health checks.".to_owned()),
        0.82,
    ));
    save_managed_stream_helper_metadata(
        root,
        &ManagedStreamHelperMetadata {
            installed_at_unix_seconds: Some(current_unix_seconds()),
            downloader_version: Some(downloader_version),
            js_runtime_version: Some(js_runtime_version),
        },
    )?;

    Ok(format!(
        "Installed managed stream helper into '{}'.",
        bin_dir.display()
    ))
}

#[cfg(test)]
pub(in crate::app::stream_support) fn import_managed_stream_helper_downloader(
    root: &Path,
    source_path: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_downloader_with_progress(root, source_path, |_| {})
}

pub(in crate::app) fn import_managed_stream_helper_downloader_with_progress<F>(
    root: &Path,
    source_path: &Path,
    progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    import_managed_stream_helper_component(
        root,
        source_path,
        ManagedStreamHelperComponent::Downloader,
        progress,
    )
}

#[cfg(test)]
pub(in crate::app::stream_support) fn import_managed_stream_helper_js_runtime(
    root: &Path,
    source_path: &Path,
) -> Result<String, String> {
    import_managed_stream_helper_js_runtime_with_progress(root, source_path, |_| {})
}

pub(in crate::app) fn import_managed_stream_helper_js_runtime_with_progress<F>(
    root: &Path,
    source_path: &Path,
    progress: F,
) -> Result<String, String>
where
    F: FnMut(StreamHelperRemediationProgress),
{
    import_managed_stream_helper_component(
        root,
        source_path,
        ManagedStreamHelperComponent::JsRuntime,
        progress,
    )
}

fn import_managed_stream_helper_component(
    root: &Path,
    source_path: &Path,
    component: ManagedStreamHelperComponent,
    mut progress: impl FnMut(StreamHelperRemediationProgress),
) -> Result<String, String> {
    if !source_path.is_file() {
        return Err(format!(
            "{} import failed because '{}' is not a file.",
            component.display_name(),
            source_path.display()
        ));
    }

    let bin_dir = managed_stream_helper_bin_dir(root);
    progress(StreamHelperRemediationProgress::new(
        format!("Preparing {}", component.display_name()),
        Some(format!(
            "Copying '{}' into '{}'.",
            source_path.display(),
            bin_dir.display()
        )),
        0.12,
    ));
    fs::create_dir_all(&bin_dir).map_err(|error| {
        format!(
            "failed to create managed stream helper directory '{}': {error}",
            bin_dir.display()
        )
    })?;

    let target_path = bin_dir.join(component.target_file_name());
    progress(StreamHelperRemediationProgress::new(
        format!("Importing {}", component.display_name()),
        Some(format!("Writing '{}'.", target_path.display())),
        0.38,
    ));
    let version = if target_path == source_path {
        probe_executable_version(&target_path, &["--version"]).map_err(|error| {
            format!(
                "{} could not be executed from '{}': {error}",
                component.display_name(),
                target_path.display()
            )
        })?
    } else {
        replace_managed_helper_executable_from_path(source_path, &target_path).and_then(|_| {
            probe_executable_version(&target_path, &["--version"]).map_err(|error| {
                let _ = fs::remove_file(&target_path);
                format!(
                    "{} could not be executed after import to '{}': {error}",
                    component.display_name(),
                    target_path.display()
                )
            })
        })?
    };

    progress(StreamHelperRemediationProgress::new(
        format!("Validating {}", component.display_name()),
        Some("Checking that the imported helper binary can be executed.".to_owned()),
        0.64,
    ));
    let mut metadata = load_managed_stream_helper_metadata(root).unwrap_or_default();
    metadata.installed_at_unix_seconds = Some(current_unix_seconds());
    component.assign_version(&mut metadata, version);
    progress(StreamHelperRemediationProgress::new(
        "Saving stream helper metadata",
        Some("Updating the managed helper inventory after import.".to_owned()),
        0.78,
    ));
    save_managed_stream_helper_metadata(root, &metadata)?;

    Ok(format!(
        "Imported {} into '{}'.",
        component.display_name(),
        target_path.display()
    ))
}

pub(in crate::app::stream_support) fn validate_installed_stream_helper_component(
    path: &Path,
    component: ManagedStreamHelperComponent,
) -> Result<String, String> {
    probe_executable_version(path, &["--version"]).map_err(|error| {
        let _ = fs::remove_file(path);
        format!(
            "{} could not be executed after install to '{}': {error}",
            component.display_name(),
            path.display()
        )
    })
}

fn replace_managed_helper_executable_from_path(
    source_path: &Path,
    target_path: &Path,
) -> Result<(), String> {
    let temp_path = target_path.with_extension(format!(
        "{}.importing",
        target_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("tmp")
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source_path, &temp_path).map_err(|error| {
        format!(
            "failed to copy '{}' into '{}': {error}",
            source_path.display(),
            temp_path.display()
        )
    })?;
    make_copied_helper_executable(&temp_path)?;
    if target_path.exists() {
        fs::remove_file(target_path).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!(
                "failed to replace existing managed helper '{}': {error}",
                target_path.display()
            )
        })?;
    }
    fs::rename(&temp_path, target_path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to move imported helper into '{}': {error}",
            target_path.display()
        )
    })
}

fn make_copied_helper_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .map_err(|error| {
                format!(
                    "failed to read helper permissions from '{}': {error}",
                    path.display()
                )
            })?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|error| {
            format!(
                "failed to mark imported helper '{}' as executable: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn extract_deno_executable_from_zip(bytes: &[u8], target_path: &Path) -> Result<(), String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)
        .map_err(|error| format!("failed to open downloaded Deno archive: {error}"))?;
    let mut found = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to read downloaded Deno archive entry: {error}"))?;
        let name = entry.name().to_ascii_lowercase();
        if !name.ends_with("deno.exe") {
            continue;
        }

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|error| format!("failed to extract Deno executable from archive: {error}"))?;
        fs::write(target_path, data).map_err(|error| {
            format!(
                "failed to write extracted Deno executable '{}': {error}",
                target_path.display()
            )
        })?;
        found = true;
        break;
    }
    if !found {
        return Err("downloaded Deno archive did not contain deno.exe".to_owned());
    }
    Ok(())
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
