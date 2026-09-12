//! Shared bounds and installation ownership for Sorotte's four managed helpers.
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

mod download;
mod install;
mod worker;
pub(super) use download::download_to_path;
pub(super) use install::{ToolInstall, extract_executable};
pub(super) use worker::HelperWorker;

#[cfg(test)]
mod process_fault_tests;
#[cfg(test)]
pub(in crate::app) mod tests;

pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HelperTool {
    YtDlp,
    Deno,
    Ffmpeg,
    Ffprobe,
}

impl HelperTool {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Deno => "Deno",
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    pub(super) fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::YtDlp => &["--ignore-config", "--version"],
            Self::Deno => &["--version"],
            Self::Ffmpeg | Self::Ffprobe => &["-version"],
        }
    }

    pub(super) fn probe(self, path: &Path, cancel: Option<&AtomicBool>) -> Result<String, String> {
        self.probe_with_args(path, self.version_args(), cancel)
    }

    pub(super) fn probe_with_args(
        self,
        path: &Path,
        args: &[&str],
        cancel: Option<&AtomicBool>,
    ) -> Result<String, String> {
        let output =
            sorotte_media_match::run_tool_probe(self.label(), path, args, PROBE_TIMEOUT, cancel)
                .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{} exited with status {}: {}",
                self.label(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.parse_version(&output.stdout)
    }

    pub(super) fn parse_version(self, stdout: &[u8]) -> Result<String, String> {
        let line = stdout
            .split(|byte| *byte == b'\n')
            .find(|line| !line.iter().all(u8::is_ascii_whitespace))
            .ok_or_else(|| "version output was empty".to_owned())?;
        let line = std::str::from_utf8(line)
            .map_err(|_| "version banner was not valid UTF-8".to_owned())?
            .trim();
        let valid = match self {
            Self::YtDlp => {
                let mut parts = line.split('.');
                let year = parts
                    .next()
                    .filter(|part| part.len() == 4)
                    .and_then(|part| part.parse::<u32>().ok());
                let month = parts.next().and_then(|part| part.parse::<u32>().ok());
                let day = parts.next().and_then(|part| part.parse::<u32>().ok());
                year.is_some_and(|year| year >= 2020)
                    && month.is_some_and(|month| (1..=12).contains(&month))
                    && day.is_some_and(|day| (1..=31).contains(&day))
                    && !line.bytes().any(|byte| byte.is_ascii_whitespace())
            }
            Self::Deno => line
                .strip_prefix("deno ")
                .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit())),
            Self::Ffmpeg => line
                .strip_prefix("ffmpeg version ")
                .is_some_and(|version| !version.trim().is_empty()),
            Self::Ffprobe => line
                .strip_prefix("ffprobe version ")
                .is_some_and(|version| !version.trim().is_empty()),
        };
        if !valid {
            return Err(format!(
                "output did not contain a valid {} version banner",
                self.label()
            ));
        }
        Ok(line.to_owned())
    }
}

pub(super) fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err("Helper operation cancelled.".to_owned())
    } else {
        Ok(())
    }
}
