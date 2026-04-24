use super::*;

const LEGACY_FOLDER_SEARCH_FIRST_FILE_TIMEOUT_SECONDS_DEFAULT: f64 = 25.0;
const LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT: f64 = 20.0;
const LEGACY_FOLDER_SEARCH_WARNING_THRESHOLD_SECONDS_DEFAULT: f64 = 2.0;
const LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT: f64 = 30.0;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LegacyStartupMediaSearchResolution {
    pub(crate) file: Option<String>,
    pub(crate) warning_lines: Vec<String>,
}

pub(crate) fn apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible(
    overrides: &mut LegacyClientArgOverrides,
    settings: Option<&StoredClientSettingsMvp>,
) {
    let resolution = resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible(
        overrides.file.as_deref(),
        settings,
    );
    for line in resolution.warning_lines {
        eprintln!("{line}");
    }
    overrides.file = resolution.file;
}

pub(crate) fn resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible(
    requested_file: Option<&str>,
    settings: Option<&StoredClientSettingsMvp>,
) -> LegacyStartupMediaSearchResolution {
    let Some(requested_file) = requested_file
        .map(str::trim)
        .filter(|file| !file.is_empty())
    else {
        return LegacyStartupMediaSearchResolution::default();
    };

    let requested_path = Path::new(requested_file);
    if requested_file.contains("://")
        || requested_file.starts_with("magnet:")
        || requested_path.is_file()
        || requested_path.is_absolute()
    {
        return LegacyStartupMediaSearchResolution {
            file: Some(requested_file.to_owned()),
            warning_lines: Vec::new(),
        };
    }

    let Some(settings) = settings else {
        return LegacyStartupMediaSearchResolution {
            file: Some(requested_file.to_owned()),
            warning_lines: Vec::new(),
        };
    };
    let Some(media_directories) = settings.media_search_directories.as_ref() else {
        return LegacyStartupMediaSearchResolution {
            file: Some(requested_file.to_owned()),
            warning_lines: Vec::new(),
        };
    };

    let simple_file_name = requested_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name == requested_file);

    let first_file_timeout_seconds = settings
        .folder_search_first_file_timeout_seconds
        .unwrap_or(LEGACY_FOLDER_SEARCH_FIRST_FILE_TIMEOUT_SECONDS_DEFAULT);
    let folder_timeout_seconds = settings
        .folder_search_timeout_seconds
        .unwrap_or(LEGACY_FOLDER_SEARCH_TIMEOUT_SECONDS_DEFAULT);
    let warning_threshold_seconds = settings
        .folder_search_warning_threshold_seconds
        .unwrap_or(LEGACY_FOLDER_SEARCH_WARNING_THRESHOLD_SECONDS_DEFAULT);
    let warning_repeat_interval_seconds = settings
        .folder_search_double_check_interval_seconds
        .unwrap_or(LEGACY_FOLDER_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS_DEFAULT);

    let mut warning_lines = Vec::new();
    for directory in media_directories
        .iter()
        .map(String::as_str)
        .filter(|directory| !directory.trim().is_empty())
    {
        let directory_path = Path::new(directory);
        if !directory_path.is_dir() {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories ignored missing media directory '{directory}'",
            ));
            continue;
        }

        let direct_candidate = directory_path.join(requested_file);
        if direct_candidate.is_file() {
            return LegacyStartupMediaSearchResolution {
                file: Some(direct_candidate.to_string_lossy().into_owned()),
                warning_lines,
            };
        }

        let Some(simple_file_name) = simple_file_name else {
            continue;
        };

        if first_file_timeout_seconds <= 0.0 {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories skipped recursive startup-file search in '{directory}' because folderSearchFirstFileTimeout is 0",
            ));
            continue;
        }

        let first_probe_started = Instant::now();
        if std::fs::read_dir(directory_path).is_err() {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories could not access media directory '{directory}'",
            ));
            continue;
        }
        if first_probe_started.elapsed().as_secs_f64() > first_file_timeout_seconds {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories skipped recursive startup-file search in '{directory}' because folderSearchFirstFileTimeout was exceeded",
            ));
            continue;
        }

        if folder_timeout_seconds <= 0.0 {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories aborted recursive startup-file search in '{directory}' because folderSearchTimeout is 0",
            ));
            continue;
        }

        if let Some(found_path) = search_media_directories_for_startup_file_name_legacy_compatible(
            directory_path,
            simple_file_name,
            folder_timeout_seconds,
            warning_threshold_seconds,
            warning_repeat_interval_seconds,
            &mut warning_lines,
        ) {
            return LegacyStartupMediaSearchResolution {
                file: Some(found_path.to_string_lossy().into_owned()),
                warning_lines,
            };
        }
    }

    LegacyStartupMediaSearchResolution {
        file: Some(requested_file.to_owned()),
        warning_lines,
    }
}

fn search_media_directories_for_startup_file_name_legacy_compatible(
    root_directory: &Path,
    target_file_name: &str,
    folder_timeout_seconds: f64,
    warning_threshold_seconds: f64,
    warning_repeat_interval_seconds: f64,
    warning_lines: &mut Vec<String>,
) -> Option<PathBuf> {
    let search_started = Instant::now();
    let mut last_warning_elapsed_seconds = None::<f64>;
    let mut scanned_file_count = 0usize;
    let mut pending_directories = vec![root_directory.to_path_buf()];
    let warning_repeat_interval_seconds = warning_repeat_interval_seconds.max(1.0);

    while let Some(directory) = pending_directories.pop() {
        if search_started.elapsed().as_secs_f64() > folder_timeout_seconds {
            warning_lines.push(format!(
                "warning: legacy mediaSearchDirectories aborted recursive startup-file search in '{}' after scanning {} file(s) because folderSearchTimeout was reached",
                root_directory.display(),
                scanned_file_count
            ));
            return None;
        }

        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let elapsed_seconds = search_started.elapsed().as_secs_f64();
            if elapsed_seconds > folder_timeout_seconds {
                warning_lines.push(format!(
                    "warning: legacy mediaSearchDirectories aborted recursive startup-file search in '{}' after scanning {} file(s) because folderSearchTimeout was reached",
                    root_directory.display(),
                    scanned_file_count
                ));
                return None;
            }
            if elapsed_seconds > warning_threshold_seconds
                && last_warning_elapsed_seconds
                    .is_none_or(|last| elapsed_seconds - last >= warning_repeat_interval_seconds)
            {
                warning_lines.push(format!(
                    "warning: legacy mediaSearchDirectories has scanned {} file(s) in '{}' for {} second(s) while resolving the startup file",
                    scanned_file_count,
                    root_directory.display(),
                    elapsed_seconds.floor() as u64
                ));
                last_warning_elapsed_seconds = Some(elapsed_seconds);
            }

            let Ok(entry) = entry else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending_directories.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            scanned_file_count = scanned_file_count.saturating_add(1);
            let file_name = entry.file_name();
            if file_name
                .to_str()
                .is_some_and(|name| name == target_file_name)
            {
                return Some(entry.path());
            }
        }
    }

    None
}
