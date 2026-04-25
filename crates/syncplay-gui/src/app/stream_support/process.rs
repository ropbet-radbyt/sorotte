use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use reqwest::blocking::Client;

use super::{STREAM_HELPER_DOWNLOAD_TIMEOUT, STREAM_HELPER_USER_AGENT};

pub(in crate::app::stream_support) fn helper_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(STREAM_HELPER_DOWNLOAD_TIMEOUT)
        .user_agent(STREAM_HELPER_USER_AGENT)
        .build()
        .map_err(|error| format!("failed to build stream-helper HTTP client: {error}"))
}

pub(in crate::app::stream_support) fn download_bytes(
    client: &Client,
    url: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("failed to download '{url}': {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to download '{url}': HTTP {}",
            response.status()
        ));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("failed reading '{url}' response body: {error}"))
}

pub(in crate::app::stream_support) fn download_to_path(
    client: &Client,
    url: &str,
    path: &Path,
) -> Result<(), String> {
    let bytes = download_bytes(client, url)?;
    fs::write(path, bytes).map_err(|error| {
        format!(
            "failed to write downloaded stream helper file '{}': {error}",
            path.display()
        )
    })
}

pub(in crate::app::stream_support) fn probe_executable_version(
    path: &Path,
    args: &[&str],
) -> Result<String, String> {
    let output = Command::new(path)
        .args(args)
        .output()
        .map_err(|error| format!("failed to start '{}': {error}", path.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        return Err(detail);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(version)
}

pub(in crate::app::stream_support) fn find_executable_on_path(
    candidates: &[&str],
) -> Option<PathBuf> {
    let path_env = env::var_os("PATH")?;
    for directory in env::split_paths(&path_env) {
        for candidate in candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
