#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    env, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
};

const GUI_EXE: &str = "sorotte-gui.exe";
const UPDATER_EXE: &str = "sorotte-gui-updater.exe";

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdaterArgs {
    pid: u32,
    source_dir: PathBuf,
    target_dir: PathBuf,
    target_exe: String,
    backup_dir: PathBuf,
    log_path: PathBuf,
    restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementPlan {
    pid: u32,
    source_dir: PathBuf,
    target_dir: PathBuf,
    target_exe_path: PathBuf,
    backup_dir: PathBuf,
    log_path: PathBuf,
    restart: bool,
    files: Vec<ReplacementFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementFile {
    source: PathBuf,
    target: PathBuf,
    backup: PathBuf,
}

fn main() -> ExitCode {
    match parse_args(env::args().skip(1)).and_then(run_update) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run_update(args: UpdaterArgs) -> Result<(), String> {
    let plan = replacement_plan(args)?;
    append_log(&plan.log_path, "waiting for Sorotte GUI to exit")?;
    wait_for_process_exit(plan.pid)?;
    append_log(&plan.log_path, "applying staged update")?;
    apply_replacement_plan(&plan)?;
    if plan.restart {
        append_log(&plan.log_path, "restarting Sorotte GUI")?;
        Command::new(&plan.target_exe_path)
            .spawn()
            .map_err(|error| format!("failed to restart Sorotte GUI: {error}"))?;
    }
    append_log(&plan.log_path, "update completed")?;
    Ok(())
}

fn parse_args<I>(args: I) -> Result<UpdaterArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut pid = None;
    let mut source_dir = None;
    let mut target_dir = None;
    let mut target_exe = None;
    let mut backup_dir = None;
    let mut log_path = None;
    let mut restart = false;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pid" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--pid requires a value".to_owned())?;
                pid = Some(
                    value
                        .parse::<u32>()
                        .map_err(|error| format!("--pid must be a process id: {error}"))?,
                );
            }
            "--source-dir" => {
                source_dir = Some(PathBuf::from(required_value(&mut args, "--source-dir")?));
            }
            "--target-dir" => {
                target_dir = Some(PathBuf::from(required_value(&mut args, "--target-dir")?));
            }
            "--target-exe" => {
                target_exe = Some(required_value(&mut args, "--target-exe")?);
            }
            "--backup-dir" => {
                backup_dir = Some(PathBuf::from(required_value(&mut args, "--backup-dir")?));
            }
            "--log" => {
                log_path = Some(PathBuf::from(required_value(&mut args, "--log")?));
            }
            "--restart" => restart = true,
            other => return Err(format!("unknown updater argument {other:?}")),
        }
    }

    Ok(UpdaterArgs {
        pid: pid.ok_or_else(|| "--pid is required".to_owned())?,
        source_dir: source_dir.ok_or_else(|| "--source-dir is required".to_owned())?,
        target_dir: target_dir.ok_or_else(|| "--target-dir is required".to_owned())?,
        target_exe: target_exe.ok_or_else(|| "--target-exe is required".to_owned())?,
        backup_dir: backup_dir.ok_or_else(|| "--backup-dir is required".to_owned())?,
        log_path: log_path.ok_or_else(|| "--log is required".to_owned())?,
        restart,
    })
}

fn required_value<I>(args: &mut I, name: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} requires a non-empty value"))
}

fn replacement_plan(args: UpdaterArgs) -> Result<ReplacementPlan, String> {
    if !args.source_dir.is_dir() {
        return Err(format!(
            "staged source directory does not exist: {}",
            args.source_dir.display()
        ));
    }
    if !args.source_dir.join(GUI_EXE).is_file() {
        return Err(format!(
            "staged source directory is missing {GUI_EXE}: {}",
            args.source_dir.display()
        ));
    }
    if !args.source_dir.join(UPDATER_EXE).is_file() {
        return Err(format!(
            "staged source directory is missing {UPDATER_EXE}: {}",
            args.source_dir.display()
        ));
    }
    if args.target_exe != GUI_EXE {
        return Err(format!(
            "target executable must be {GUI_EXE}, got {}",
            args.target_exe
        ));
    }
    let target_exe_path = args.target_dir.join(&args.target_exe);
    if !target_exe_path.is_file() {
        return Err(format!(
            "target executable does not exist: {}",
            target_exe_path.display()
        ));
    }
    let source_files = collect_source_files(&args.source_dir)?;
    let files = source_files
        .into_iter()
        .map(|source| {
            let relative = source.strip_prefix(&args.source_dir).map_err(|error| {
                format!(
                    "failed to derive staged relative path for {}: {error}",
                    source.display()
                )
            })?;
            if !relative_path_is_safe(relative) {
                return Err(format!(
                    "staged file path is unsafe: {}",
                    relative.display()
                ));
            }
            Ok(ReplacementFile {
                source: source.clone(),
                target: args.target_dir.join(relative),
                backup: args.backup_dir.join(relative),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(ReplacementPlan {
        pid: args.pid,
        source_dir: args.source_dir,
        target_dir: args.target_dir,
        target_exe_path,
        backup_dir: args.backup_dir,
        log_path: args.log_path,
        restart: args.restart,
        files,
    })
}

fn collect_source_files(source_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![source_dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path)
            .map_err(|error| format!("failed to read directory {}: {error}", path.display()))?
        {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read directory entry in {}: {error}",
                    path.display()
                )
            })?;
            let entry_path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "failed to inspect directory entry {}: {error}",
                    entry_path.display()
                )
            })?;
            if file_type.is_dir() {
                pending.push(entry_path);
            } else if file_type.is_file() {
                files.push(entry_path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn relative_path_is_safe(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn apply_replacement_plan(plan: &ReplacementPlan) -> Result<(), String> {
    fs::create_dir_all(&plan.backup_dir).map_err(|error| {
        format!(
            "failed to create backup directory {}: {error}",
            plan.backup_dir.display()
        )
    })?;
    for file in &plan.files {
        if let Some(parent) = file.target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create target directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        if let Some(parent) = file.backup.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create backup directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        if file.target.exists() {
            fs::copy(&file.target, &file.backup).map_err(|error| {
                format!(
                    "failed to back up {} to {}: {error}",
                    file.target.display(),
                    file.backup.display()
                )
            })?;
        }
        fs::copy(&file.source, &file.target).map_err(|error| {
            format!(
                "failed to replace {} from {}: {error}",
                file.target.display(),
                file.source.display()
            )
        })?;
    }
    Ok(())
}

fn append_log(path: &Path, message: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create updater log directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open updater log {}: {error}", path.display()))?;
    writeln!(file, "{message}")
        .map_err(|error| format!("failed to write updater log {}: {error}", path.display()))
}

#[cfg(windows)]
fn wait_for_process_exit(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    // SAFETY: OpenProcess is called with SYNCHRONIZE only, the returned handle is checked for null,
    // waited on without dereferencing, and closed exactly once with CloseHandle.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return Ok(());
    }
    // SAFETY: handle is a valid process handle returned by OpenProcess and remains open until
    // CloseHandle below.
    unsafe {
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
    }
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_process_exit(_pid: u32) -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(250));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_requires_core_paths() {
        let error = parse_args(["--pid".to_owned(), "123".to_owned()])
            .expect_err("missing paths should fail");
        assert!(error.contains("--source-dir"));
    }

    #[test]
    fn parse_args_accepts_restart_flag() {
        let args = parse_args([
            "--pid".to_owned(),
            "123".to_owned(),
            "--source-dir".to_owned(),
            "stage".to_owned(),
            "--target-dir".to_owned(),
            "target".to_owned(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--backup-dir".to_owned(),
            "backup".to_owned(),
            "--log".to_owned(),
            "update.log".to_owned(),
            "--restart".to_owned(),
        ])
        .expect("valid args should parse");

        assert_eq!(args.pid, 123);
        assert!(args.restart);
    }

    #[test]
    fn relative_path_rejects_parent_components() {
        assert!(relative_path_is_safe(Path::new("sorotte-gui.exe")));
        assert!(!relative_path_is_safe(Path::new("../sorotte-gui.exe")));
    }
}
