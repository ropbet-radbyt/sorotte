use super::*;

fn push_unique_pathbuf_legacy_compatible(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

#[cfg(windows)]
fn managed_mpv_launch_candidate_file_names_legacy_compatible() -> &'static [&'static str] {
    &["mpv.exe", "mpv.com"]
}

#[cfg(not(windows))]
fn managed_mpv_launch_candidate_file_names_legacy_compatible() -> &'static [&'static str] {
    &["mpv"]
}

pub(crate) fn resolve_managed_mpv_launch_program_legacy_compatible(requested: &Path) -> PathBuf {
    let mut candidates = vec![requested.to_path_buf()];
    if requested.is_dir() || !requested.exists() {
        for file_name in managed_mpv_launch_candidate_file_names_legacy_compatible() {
            push_unique_pathbuf_legacy_compatible(&mut candidates, requested.join(file_name));
        }
    }
    if !requested.exists()
        && let Some(parent) = requested.parent()
        && let Some(file_name) = requested.file_name().and_then(|value| value.to_str())
    {
        let normalized = file_name.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "mpv" | "mpv.exe" | "mpv.com") {
            for candidate_file_name in managed_mpv_launch_candidate_file_names_legacy_compatible() {
                push_unique_pathbuf_legacy_compatible(
                    &mut candidates,
                    parent.join(candidate_file_name),
                );
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| requested.to_path_buf())
}

pub(crate) fn managed_mpv_launch_program_requires_existing_file_legacy_compatible(
    path: &Path,
) -> bool {
    path.is_absolute()
        || path
            .to_string_lossy()
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
}

pub(crate) fn find_default_managed_mpv_bin() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors().take(6) {
        let mpv_dir = ancestor.join("mpv");
        #[cfg(windows)]
        {
            let exe = mpv_dir.join("mpv.exe");
            if exe.exists() {
                return Some(exe);
            }
            let com = mpv_dir.join("mpv.com");
            if com.exists() {
                return Some(com);
            }
        }
        #[cfg(not(windows))]
        {
            let bin = mpv_dir.join("mpv");
            if bin.exists() {
                return Some(bin);
            }
        }
    }
    None
}
