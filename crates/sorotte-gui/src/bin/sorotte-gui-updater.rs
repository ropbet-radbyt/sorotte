#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::BTreeSet,
    env, fs,
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

const GUI_EXE: &str = "sorotte-gui.exe";
const UPDATER_EXE: &str = "sorotte-gui-updater.exe";
#[cfg(all(debug_assertions, feature = "updater-integration-test"))]
const UPDATER_INTEGRATION_ALLOW_ELEVATED_ENV: &str =
    "SOROTTE_UPDATER_INTEGRATION_TEST_ALLOW_ELEVATED";
const INSTALL_MANIFEST: &str = "sorotte-install.json";
const INSTALL_MANIFEST_SCHEMA: &str = "sorotte-gui-install-manifest-v2";
const INSTALL_TARGET: &str = "windows-x86_64";
const JOURNAL_FILE: &str = ".sorotte-update-journal-v1.jsonl";
const JOURNAL_SCHEMA: &str = "sorotte-update-replacement-journal-v1";
const LEGACY_MANAGED_FILES: &[&str] = &[
    GUI_EXE,
    UPDATER_EXE,
    "README.md",
    "LICENSE",
    "resources/sorotte_syncplayintf.lua",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdaterArgs {
    pid: u32,
    input: Option<UpdateInput>,
    target_dir: PathBuf,
    target_exe: String,
    log_path: PathBuf,
    restart: bool,
    detached_helper_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateInput {
    Package {
        package: PathBuf,
        package_sha256: String,
    },
    LegacySource {
        source_dir: PathBuf,
        backup_dir: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdaterExecutionLocation {
    InstalledBootstrap,
    DetachedHelper,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallFile {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    #[serde(default)]
    schema: String,
    app: String,
    #[serde(default)]
    version: String,
    target: String,
    #[serde(default)]
    files: Vec<InstallFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementPlan {
    target_dir: PathBuf,
    target_exe_path: PathBuf,
    journal_path: PathBuf,
    log_path: PathBuf,
    restart: bool,
    files: Vec<ReplacementFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplacementFile {
    relative: PathBuf,
    source: Option<PathBuf>,
    target: PathBuf,
    temporary: PathBuf,
    backup: PathBuf,
    original_sha256: Option<String>,
    expected_sha256: Option<String>,
    target_existed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplacementJournal {
    schema: String,
    entries: Vec<ReplacementJournalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplacementJournalEntry {
    relative: PathBuf,
    temporary: PathBuf,
    backup: PathBuf,
    target_existed: bool,
    original_sha256: Option<String>,
    replacement_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalCommit {
    committed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyProgress {
    BeforePrepare(usize),
    BeforeReplace(usize),
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
    let integration_test_override = updater_integration_test_allows_elevated_process(&args);
    run_update_with_elevation_check(args, move || {
        if integration_test_override {
            Ok(false)
        } else {
            process_is_elevated()
        }
    })
}

#[cfg(all(debug_assertions, feature = "updater-integration-test"))]
fn updater_integration_test_allows_elevated_process(args: &UpdaterArgs) -> bool {
    env::var_os(UPDATER_INTEGRATION_ALLOW_ELEVATED_ENV).as_deref()
        == Some(std::ffi::OsStr::new("1"))
        && args.target_dir.starts_with(env::temp_dir())
}

#[cfg(not(all(debug_assertions, feature = "updater-integration-test")))]
fn updater_integration_test_allows_elevated_process(_args: &UpdaterArgs) -> bool {
    false
}

fn run_update_with_elevation_check<F>(args: UpdaterArgs, elevation_check: F) -> Result<(), String>
where
    F: FnOnce() -> Result<bool, String>,
{
    run_update_with_checks(args, elevation_check, validate_updater_location)
}

fn run_update_with_checks<F, G>(
    args: UpdaterArgs,
    elevation_check: F,
    updater_location_check: G,
) -> Result<(), String>
where
    F: FnOnce() -> Result<bool, String>,
    G: FnOnce(&UpdaterArgs) -> Result<UpdaterExecutionLocation, String>,
{
    if args.target_exe != GUI_EXE {
        return Err(format!("target executable must be {GUI_EXE}"));
    }
    if elevation_check()? {
        let error = "Sorotte refuses to run automatic replacement from an elevated updater process. Install this release manually from a trusted package.".to_owned();
        let _ = append_log(&args.log_path, &error);
        return Err(error);
    }
    if matches!(
        updater_location_check(&args)?,
        UpdaterExecutionLocation::InstalledBootstrap
    ) {
        return launch_detached_update_helper(&args);
    }
    append_log(&args.log_path, "waiting for Sorotte GUI to exit")?;
    wait_for_process_exit(args.pid)?;
    recover_pending_update_with_retry(&args.target_dir)?;

    let Some(input) = args.input.clone() else {
        if args.restart {
            let target_exe_path = args.target_dir.join(&args.target_exe);
            append_log(
                &args.log_path,
                "restarting Sorotte GUI after update recovery",
            )?;
            Command::new(&target_exe_path)
                .spawn()
                .map_err(|error| format!("failed to restart Sorotte GUI: {error}"))?;
        }
        append_log(&args.log_path, "interrupted update recovery completed")?;
        return Ok(());
    };

    match input {
        UpdateInput::Package {
            package,
            package_sha256,
        } => {
            // Read and authenticate one immutable in-memory snapshot after the GUI exits.
            // Extraction never reopens the user-writable package, closing the substitution window.
            let package_bytes = read_verified_package_bytes(&package, &package_sha256)?;
            let staging_dir = create_protected_staging_dir(&args.target_dir)?;
            let update_result = (|| {
                extract_zip_bytes_safe(&package_bytes, &staging_dir)?;
                apply_validated_source_update(&args, &staging_dir)
            })();
            let cleanup_result = remove_directory_if_exists(&staging_dir);
            match (update_result, cleanup_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(format!("{error}; additionally {cleanup}")),
            }
        }
        UpdateInput::LegacySource {
            source_dir,
            backup_dir: _,
        } => {
            validate_legacy_source_root(&source_dir)?;
            apply_validated_source_update(&args, &source_dir)
        }
    }
}

fn apply_validated_source_update(args: &UpdaterArgs, source_dir: &Path) -> Result<(), String> {
    let new_manifest = read_install_manifest(&source_dir.join(INSTALL_MANIFEST))?;
    validate_extracted_package(source_dir, &new_manifest)?;
    let old_manifest = read_install_manifest(&args.target_dir.join(INSTALL_MANIFEST))?;
    let plan = replacement_plan(args, source_dir, &new_manifest, Some(&old_manifest))?;
    append_log(&plan.log_path, "applying authenticated staged update")?;
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
    let mut package = None;
    let mut package_sha256 = None;
    let mut source_dir = None;
    let mut backup_dir = None;
    let mut target_dir = None;
    let mut target_exe = None;
    let mut log_path = None;
    let mut restart = false;
    let mut recover = false;
    let mut detached_helper_sha256 = None;

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
            "--package" => package = Some(PathBuf::from(required_value(&mut args, "--package")?)),
            "--package-sha256" => {
                package_sha256 = Some(required_value(&mut args, "--package-sha256")?)
            }
            "--source-dir" => {
                source_dir = Some(PathBuf::from(required_value(&mut args, "--source-dir")?));
            }
            "--backup-dir" => {
                backup_dir = Some(PathBuf::from(required_value(&mut args, "--backup-dir")?));
            }
            "--target-dir" => {
                target_dir = Some(PathBuf::from(required_value(&mut args, "--target-dir")?));
            }
            "--target-exe" => target_exe = Some(required_value(&mut args, "--target-exe")?),
            "--log" => log_path = Some(PathBuf::from(required_value(&mut args, "--log")?)),
            "--restart" => restart = true,
            "--recover" => recover = true,
            "--detached-helper-sha256" => {
                detached_helper_sha256 =
                    Some(required_value(&mut args, "--detached-helper-sha256")?)
            }
            other => return Err(format!("unknown updater argument {other:?}")),
        }
    }

    let input = match (recover, package, package_sha256, source_dir, backup_dir) {
        (true, None, None, None, None) => None,
        (false, Some(package), Some(package_sha256), None, None) => {
            validate_sha256_hex(&package_sha256)?;
            Some(UpdateInput::Package {
                package,
                package_sha256,
            })
        }
        (false, None, None, Some(source_dir), Some(backup_dir)) => {
            Some(UpdateInput::LegacySource {
                source_dir,
                backup_dir,
            })
        }
        (false, None, None, None, None) => {
            return Err("--package and --package-sha256 are required".to_owned());
        }
        _ => {
            return Err(
                "use --recover alone, --package with --package-sha256, or the exact legacy --source-dir with --backup-dir argument pair"
                    .to_owned(),
            );
        }
    };
    if let Some(expected_sha256) = detached_helper_sha256.as_deref() {
        validate_sha256_hex(expected_sha256)?;
    }
    Ok(UpdaterArgs {
        pid: pid.ok_or_else(|| "--pid is required".to_owned())?,
        input,
        target_dir: target_dir.ok_or_else(|| "--target-dir is required".to_owned())?,
        target_exe: target_exe.ok_or_else(|| "--target-exe is required".to_owned())?,
        log_path: log_path.ok_or_else(|| "--log is required".to_owned())?,
        restart,
        detached_helper_sha256,
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

#[cfg(windows)]
fn process_is_elevated() -> Result<bool, String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns the current process pseudo-handle. OpenProcessToken writes
    // one owned token handle to `token`, which is checked and closed exactly once below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!(
            "failed opening updater process token: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned_length = 0u32;
    // SAFETY: `token` is a valid query handle, and the output pointer/length describe the live
    // TOKEN_ELEVATION value for the duration of this call.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_length,
        )
    };
    // SAFETY: token was returned by OpenProcessToken and has not previously been closed.
    unsafe {
        CloseHandle(token);
    }
    if queried == 0 {
        return Err(format!(
            "failed reading updater elevation state: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(not(windows))]
fn process_is_elevated() -> Result<bool, String> {
    Ok(false)
}

fn validate_legacy_source_root(source_dir: &Path) -> Result<(), String> {
    ensure_directory_is_not_reparse_point(source_dir)?;
    for required in [UPDATER_EXE, GUI_EXE, INSTALL_MANIFEST] {
        let path = source_dir.join(required);
        reject_reparse_path(&path)?;
        if !path.is_file() {
            return Err(format!(
                "legacy staged update is missing required file {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_updater_location(args: &UpdaterArgs) -> Result<UpdaterExecutionLocation, String> {
    ensure_directory_is_not_reparse_point(&args.target_dir)?;
    let running = fs::canonicalize(
        env::current_exe().map_err(|error| format!("failed resolving updater path: {error}"))?,
    )
    .map_err(|error| format!("failed canonicalizing updater path: {error}"))?;
    reject_reparse_path(&running)?;
    let installed = fs::canonicalize(args.target_dir.join(UPDATER_EXE)).map_err(|error| {
        format!(
            "failed resolving installed update helper {}: {error}",
            args.target_dir.join(UPDATER_EXE).display()
        )
    })?;
    reject_reparse_path(&installed)?;
    let Some(expected_detached_sha256) = args.detached_helper_sha256.as_deref() else {
        if paths_are_equal(&running, &installed) {
            return Ok(UpdaterExecutionLocation::InstalledBootstrap);
        }
        if let Some(UpdateInput::LegacySource { source_dir, .. }) = args.input.as_ref() {
            validate_legacy_source_root(source_dir)?;
            let staged_legacy_helper =
                fs::canonicalize(source_dir.join(UPDATER_EXE)).map_err(|error| {
                    format!(
                        "failed resolving staged legacy update helper {}: {error}",
                        source_dir.join(UPDATER_EXE).display()
                    )
                })?;
            if paths_are_equal(&running, &staged_legacy_helper) {
                return Ok(UpdaterExecutionLocation::DetachedHelper);
            }
        }
        return Err(format!(
            "refusing to update from an unprotected helper location {}; expected {}",
            running.display(),
            installed.display()
        ));
    };
    validate_sha256_hex(expected_detached_sha256)?;
    if paths_are_equal(&running, &installed) {
        return Err("detached updater mode cannot run from the installed helper path".to_owned());
    }
    let running_parent = running
        .parent()
        .ok_or_else(|| "detached updater path has no parent directory".to_owned())?;
    let bootstrap_root = running_parent
        .parent()
        .ok_or_else(|| "detached updater directory has no protected parent".to_owned())?;
    let canonical_target = fs::canonicalize(&args.target_dir).map_err(|error| {
        format!(
            "failed canonicalizing updater target directory {}: {error}",
            args.target_dir.display()
        )
    })?;
    if !paths_are_equal(bootstrap_root, &canonical_target)
        || !running_parent.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with(".sorotte-update-bootstrap-")
        })
    {
        return Err(format!(
            "detached updater helper {} is outside Sorotte's protected bootstrap directory",
            running.display()
        ));
    }
    require_file_digest(&running, expected_detached_sha256, "detached update helper")?;
    require_file_digest(
        &installed,
        expected_detached_sha256,
        "installed update helper",
    )?;
    Ok(UpdaterExecutionLocation::DetachedHelper)
}

fn launch_detached_update_helper(args: &UpdaterArgs) -> Result<(), String> {
    let running = env::current_exe()
        .map_err(|error| format!("failed resolving installed updater path: {error}"))?;
    reject_reparse_path(&running)?;
    let bytes = fs::read(&running).map_err(|error| {
        format!(
            "failed reading installed updater {} for detached execution: {error}",
            running.display()
        )
    })?;
    let expected_sha256 = sha256_bytes(&bytes);
    let bootstrap_dir = create_protected_bootstrap_dir(&args.target_dir)?;
    let detached_path = bootstrap_dir.join("sorotte-gui-updater-bootstrap.exe");
    let write_result = (|| {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&detached_path)
            .map_err(|error| {
                format!(
                    "failed creating detached update helper {}: {error}",
                    detached_path.display()
                )
            })?;
        output
            .write_all(&bytes)
            .and_then(|()| output.flush())
            .and_then(|()| output.sync_all())
            .map_err(|error| {
                format!(
                    "failed flushing detached update helper {}: {error}",
                    detached_path.display()
                )
            })
    })();
    if let Err(error) = write_result {
        let _ = remove_directory_if_exists(&bootstrap_dir);
        return Err(error);
    }
    if let Err(error) =
        require_file_digest(&detached_path, &expected_sha256, "detached update helper")
    {
        let _ = remove_directory_if_exists(&bootstrap_dir);
        return Err(error);
    }
    append_log(
        &args.log_path,
        "delegating replacement to a detached authenticated updater copy",
    )?;
    let mut command = Command::new(&detached_path);
    command.args(detached_update_helper_args(args, &expected_sha256));
    command.spawn().map_err(|error| {
        let _ = remove_directory_if_exists(&bootstrap_dir);
        format!(
            "failed launching detached update helper {}: {error}",
            detached_path.display()
        )
    })?;
    Ok(())
}

fn detached_update_helper_args(args: &UpdaterArgs, expected_sha256: &str) -> Vec<String> {
    let mut result = vec![
        "--pid".to_owned(),
        args.pid.to_string(),
        "--target-dir".to_owned(),
        args.target_dir.display().to_string(),
        "--target-exe".to_owned(),
        args.target_exe.clone(),
        "--log".to_owned(),
        args.log_path.display().to_string(),
        "--detached-helper-sha256".to_owned(),
        expected_sha256.to_owned(),
    ];
    match args.input.as_ref() {
        Some(UpdateInput::Package {
            package,
            package_sha256,
        }) => {
            result.push("--package".to_owned());
            result.push(package.display().to_string());
            result.push("--package-sha256".to_owned());
            result.push(package_sha256.clone());
        }
        Some(UpdateInput::LegacySource {
            source_dir,
            backup_dir,
        }) => {
            result.push("--source-dir".to_owned());
            result.push(source_dir.display().to_string());
            result.push("--backup-dir".to_owned());
            result.push(backup_dir.display().to_string());
        }
        None => result.push("--recover".to_owned()),
    }
    if args.restart {
        result.push("--restart".to_owned());
    }
    result
}

fn paths_are_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn read_verified_package_bytes(path: &Path, expected_sha256: &str) -> Result<Vec<u8>, String> {
    validate_sha256_hex(expected_sha256)?;
    reject_reparse_path(path)?;
    let bytes = fs::read(path)
        .map_err(|error| format!("failed reading update package {}: {error}", path.display()))?;
    let actual = sha256_bytes(&bytes);
    if !actual.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(format!(
            "update package SHA-256 mismatch: expected {}, got {actual}",
            expected_sha256.trim()
        ));
    }
    Ok(bytes)
}

fn validate_sha256_hex(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("update SHA-256 must be 64 hexadecimal characters".to_owned())
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    bytes_to_lower_hex(&Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed reading {} for hashing: {error}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn create_protected_staging_dir(target_dir: &Path) -> Result<PathBuf, String> {
    ensure_directory_is_not_reparse_point(target_dir)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let path = target_dir.join(format!(
        ".sorotte-update-stage-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).map_err(|error| {
        format!(
            "failed creating protected update staging directory {}: {error}",
            path.display()
        )
    })?;
    ensure_directory_is_not_reparse_point(&path)?;
    Ok(path)
}

fn create_protected_bootstrap_dir(target_dir: &Path) -> Result<PathBuf, String> {
    ensure_directory_is_not_reparse_point(target_dir)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let path = target_dir.join(format!(
        ".sorotte-update-bootstrap-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).map_err(|error| {
        format!(
            "failed creating protected updater bootstrap directory {}: {error}",
            path.display()
        )
    })?;
    ensure_directory_is_not_reparse_point(&path)?;
    Ok(path)
}

fn extract_zip_bytes_safe(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("failed opening update package: {error}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed reading update package entry {index}: {error}"))?;
        let Some(relative) = safe_relative_path(entry.name()) else {
            return Err(format!(
                "update package contains unsafe path {:?}",
                entry.name()
            ));
        };
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!(
                "update package contains a symbolic-link entry {:?}",
                entry.name()
            ));
        }
        let output = destination.join(&relative);
        if entry.is_dir() {
            create_relative_directories_without_reparse(destination, &relative)?;
            continue;
        }
        if let Some(parent) = relative.parent() {
            create_relative_directories_without_reparse(destination, parent)?;
        }
        if fs::symlink_metadata(&output).is_ok() {
            return Err(format!(
                "update package contains duplicate output path {}",
                relative.display()
            ));
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| format!("failed creating {}: {error}", output.display()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|error| format!("failed extracting {}: {error}", output.display()))?;
        file.flush()
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("failed flushing {}: {error}", output.display()))?;
    }
    Ok(())
}

fn safe_relative_path(name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || has_windows_drive_prefix(name)
    {
        return None;
    }
    let mut safe = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            _ if part.contains(':') => return None,
            _ => safe.push(part),
        }
    }
    relative_path_is_safe(&safe).then_some(safe)
}

fn has_windows_drive_prefix(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn relative_path_is_safe(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn read_install_manifest(path: &Path) -> Result<InstallManifest, String> {
    reject_reparse_path(path)?;
    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed reading install manifest {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_str(contents.trim_start_matches('\u{feff}')).map_err(|error| {
        format!(
            "failed parsing install manifest {}: {error}",
            path.display()
        )
    })
}

fn validate_extracted_package(root: &Path, manifest: &InstallManifest) -> Result<(), String> {
    if manifest.schema != INSTALL_MANIFEST_SCHEMA {
        return Err(format!(
            "unsupported install manifest schema {:?}",
            manifest.schema
        ));
    }
    if manifest.app != "sorotte-gui" || manifest.target != INSTALL_TARGET {
        return Err("update install manifest has the wrong app or target".to_owned());
    }
    let mut declared = BTreeSet::new();
    for file in &manifest.files {
        validate_sha256_hex(&file.sha256)?;
        let relative = safe_relative_path(&file.path)
            .ok_or_else(|| format!("install manifest contains unsafe path {:?}", file.path))?;
        if relative == Path::new(INSTALL_MANIFEST) || !declared.insert(relative.clone()) {
            return Err(format!(
                "install manifest contains duplicate or reserved path {}",
                relative.display()
            ));
        }
        let path = root.join(&relative);
        reject_reparse_path(&path)?;
        if !path.is_file() {
            return Err(format!("update package is missing {}", relative.display()));
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(&file.sha256) {
            return Err(format!(
                "update payload digest mismatch for {}: expected {}, got {actual}",
                relative.display(),
                file.sha256
            ));
        }
    }
    for required in [GUI_EXE, UPDATER_EXE] {
        if !declared.contains(Path::new(required)) {
            return Err(format!(
                "install manifest does not declare required file {required}"
            ));
        }
    }
    let mut actual = collect_regular_relative_files(root)?;
    actual.remove(Path::new(INSTALL_MANIFEST));
    if actual != declared {
        return Err("update package contents do not exactly match the install manifest".to_owned());
    }
    Ok(())
}

fn collect_regular_relative_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        ensure_directory_is_not_reparse_point(&directory)?;
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed reading {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!("failed reading entry in {}: {error}", directory.display())
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed inspecting {}: {error}", path.display()))?;
            if metadata_is_reparse_or_symlink(&metadata) {
                return Err(format!(
                    "update package contains a link or reparse point at {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.insert(
                    path.strip_prefix(root)
                        .map_err(|error| error.to_string())?
                        .to_path_buf(),
                );
            } else {
                return Err(format!(
                    "update package contains a non-regular file at {}",
                    path.display()
                ));
            }
        }
    }
    Ok(files)
}

fn replacement_plan(
    args: &UpdaterArgs,
    source_dir: &Path,
    new_manifest: &InstallManifest,
    old_manifest: Option<&InstallManifest>,
) -> Result<ReplacementPlan, String> {
    if args.target_exe != GUI_EXE {
        return Err(format!("target executable must be {GUI_EXE}"));
    }
    ensure_directory_is_not_reparse_point(&args.target_dir)?;
    let target_exe_path = args.target_dir.join(&args.target_exe);
    reject_reparse_path(&target_exe_path)?;
    if !target_exe_path.is_file() {
        return Err(format!(
            "target executable does not exist: {}",
            target_exe_path.display()
        ));
    }

    let transaction_id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let mut new_paths = BTreeSet::new();
    let mut files = Vec::new();
    for install_file in &new_manifest.files {
        let relative = safe_relative_path(&install_file.path)
            .ok_or_else(|| format!("unsafe install path {:?}", install_file.path))?;
        new_paths.insert(relative.clone());
        files.push(replacement_file(
            &args.target_dir,
            relative.clone(),
            Some(source_dir.join(&relative)),
            Some(install_file.sha256.clone()),
            &transaction_id,
        )?);
    }
    let marker_relative = PathBuf::from(INSTALL_MANIFEST);
    new_paths.insert(marker_relative.clone());
    files.push(replacement_file(
        &args.target_dir,
        marker_relative.clone(),
        Some(source_dir.join(&marker_relative)),
        Some(sha256_file(&source_dir.join(&marker_relative))?),
        &transaction_id,
    )?);

    if let Some(old_manifest) = old_manifest {
        if old_manifest.app != "sorotte-gui" || old_manifest.target != INSTALL_TARGET {
            return Err("existing install manifest has the wrong app or target".to_owned());
        }
        if !old_manifest.schema.is_empty() && old_manifest.schema != INSTALL_MANIFEST_SCHEMA {
            return Err(format!(
                "unsupported existing install manifest schema {:?}",
                old_manifest.schema
            ));
        }
        let old_paths = if old_manifest.files.is_empty() {
            LEGACY_MANAGED_FILES
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        } else {
            old_manifest
                .files
                .iter()
                .map(|old_file| {
                    safe_relative_path(&old_file.path).ok_or_else(|| {
                        format!(
                            "existing install manifest contains unsafe path {:?}",
                            old_file.path
                        )
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        for relative in old_paths {
            if !new_paths.contains(&relative) {
                files.push(replacement_file(
                    &args.target_dir,
                    relative,
                    None,
                    None,
                    &transaction_id,
                )?);
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(ReplacementPlan {
        target_dir: args.target_dir.clone(),
        target_exe_path,
        journal_path: args.target_dir.join(JOURNAL_FILE),
        log_path: args.log_path.clone(),
        restart: args.restart,
        files,
    })
}

fn replacement_file(
    target_dir: &Path,
    relative: PathBuf,
    source: Option<PathBuf>,
    expected_sha256: Option<String>,
    transaction_id: &str,
) -> Result<ReplacementFile, String> {
    if source.is_some() != expected_sha256.is_some() {
        return Err(
            "replacement source and digest must either both be present or both be absent"
                .to_owned(),
        );
    }
    if !relative_path_is_safe(&relative) {
        return Err(format!("unsafe replacement path {}", relative.display()));
    }
    let target = target_dir.join(&relative);
    let file_name = target
        .file_name()
        .ok_or_else(|| format!("replacement has no file name: {}", relative.display()))?
        .to_string_lossy();
    let temporary = target.with_file_name(format!(".{file_name}.sorotte-new-{transaction_id}"));
    let backup = target.with_file_name(format!(".{file_name}.sorotte-old-{transaction_id}"));
    let target_existed = match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata_is_reparse_or_symlink(&metadata) => {
            return Err(format!(
                "replacement target is a link or reparse point: {}",
                target.display()
            ));
        }
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => {
            return Err(format!(
                "replacement target is not a regular file: {}",
                target.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("failed inspecting {}: {error}", target.display())),
    };
    let original_sha256 = target_existed.then(|| sha256_file(&target)).transpose()?;
    Ok(ReplacementFile {
        relative,
        source,
        target,
        temporary,
        backup,
        original_sha256,
        expected_sha256,
        target_existed,
    })
}

fn apply_replacement_plan(plan: &ReplacementPlan) -> Result<(), String> {
    apply_replacement_plan_with_hook(plan, |_| Ok(()))
}

fn apply_replacement_plan_with_hook<F>(
    plan: &ReplacementPlan,
    mut before_progress: F,
) -> Result<(), String>
where
    F: FnMut(ApplyProgress) -> Result<(), String>,
{
    recover_pending_update(&plan.target_dir)?;
    let journal = journal_for_plan(plan)?;
    write_journal_header(&plan.journal_path, &journal)?;

    let result = (|| {
        for (index, file) in plan.files.iter().enumerate() {
            let Some(source) = file.source.as_ref() else {
                continue;
            };
            before_progress(ApplyProgress::BeforePrepare(index + 1))?;
            if let Some(parent) = file.relative.parent() {
                create_relative_directories_without_reparse(&plan.target_dir, parent)?;
            }
            reject_reparse_path(source)?;
            let bytes = fs::read(source).map_err(|error| {
                format!("failed reading staged file {}: {error}", source.display())
            })?;
            if let Some(expected) = file.expected_sha256.as_deref() {
                let actual = sha256_bytes(&bytes);
                if !actual.eq_ignore_ascii_case(expected) {
                    return Err(format!(
                        "staged file digest mismatch for {}: expected {expected}, got {actual}",
                        file.relative.display()
                    ));
                }
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file.temporary)
                .map_err(|error| {
                    format!("failed creating {}: {error}", file.temporary.display())
                })?;
            output
                .write_all(&bytes)
                .and_then(|()| output.flush())
                .and_then(|()| output.sync_all())
                .map_err(|error| {
                    format!("failed flushing {}: {error}", file.temporary.display())
                })?;
        }

        for (index, file) in plan.files.iter().enumerate() {
            if file.source.is_none() && !file.target_existed {
                continue;
            }
            before_progress(ApplyProgress::BeforeReplace(index + 1))?;
            reject_reparse_parent(&plan.target_dir, &file.target)?;
            validate_replacement_target_unchanged(file)?;
            install_prepared_replacement(file)?;
        }
        append_journal_commit(&plan.journal_path)?;
        Ok(())
    })();

    if let Err(error) = result {
        return match rollback_uncommitted_update(&plan.target_dir, &journal) {
            Ok(()) => match remove_file_if_exists(&plan.journal_path) {
                Ok(()) => Err(format!("{error}; all changed files were rolled back")),
                Err(cleanup_error) => Err(format!(
                    "{error}; all changed files were rolled back, but the validated recovery journal could not be removed: {cleanup_error}"
                )),
            },
            Err(rollback_error) => Err(format!(
                "{error}; rollback was incomplete: {rollback_error}; recovery journal retained at {}",
                plan.journal_path.display()
            )),
        };
    }

    // Failure here is deferred: the commit marker makes subsequent recovery finish cleanup rather
    // than undoing an already-complete installation. This matters when the running updater backup
    // remains image-locked until this process exits.
    let _ = cleanup_committed_update(&plan.target_dir, &journal, &plan.journal_path);
    Ok(())
}

fn validate_replacement_target_unchanged(file: &ReplacementFile) -> Result<(), String> {
    let current = regular_file_digest_if_present(&file.target, "replacement target")?;
    if current.as_deref() != file.original_sha256.as_deref() {
        return Err(format!(
            "replacement target changed during update: {}",
            file.target.display()
        ));
    }
    if regular_file_digest_if_present(&file.backup, "rollback backup")?.is_some() {
        return Err(format!(
            "rollback backup unexpectedly exists before replacement: {}",
            file.backup.display()
        ));
    }
    Ok(())
}

fn install_prepared_replacement(file: &ReplacementFile) -> Result<(), String> {
    match (file.target_existed, file.expected_sha256.as_deref()) {
        (true, Some(expected)) => {
            require_file_digest(&file.temporary, expected, "prepared replacement")?;
            atomic_replace_with_backup(&file.target, &file.temporary, &file.backup)?;
        }
        (true, None) => {
            fs::rename(&file.target, &file.backup).map_err(|error| {
                format!(
                    "failed moving obsolete target {} to rollback backup {}: {error}",
                    file.target.display(),
                    file.backup.display()
                )
            })?;
        }
        (false, Some(expected)) => {
            require_file_digest(&file.temporary, expected, "prepared replacement")?;
            fs::rename(&file.temporary, &file.target).map_err(|error| {
                format!(
                    "failed atomically installing new file {}: {error}",
                    file.target.display()
                )
            })?;
        }
        (false, None) => return Ok(()),
    }

    match file.expected_sha256.as_deref() {
        Some(expected) => require_file_digest(&file.target, expected, "installed target")?,
        None => require_file_absent(&file.target, "removed target")?,
    }
    if let Some(original) = file.original_sha256.as_deref() {
        require_file_digest(&file.backup, original, "rollback backup")?;
    }
    Ok(())
}

#[cfg(windows)]
fn atomic_replace_with_backup(
    target: &Path,
    replacement: &Path,
    backup: &Path,
) -> Result<(), String> {
    windows_replace_file(target, replacement, Some(backup)).map_err(|error| {
        format!(
            "failed atomically replacing {} with {} and backup {}: {error}",
            target.display(),
            replacement.display(),
            backup.display()
        )
    })
}

#[cfg(not(windows))]
fn atomic_replace_with_backup(
    target: &Path,
    replacement: &Path,
    backup: &Path,
) -> Result<(), String> {
    fs::rename(target, backup).map_err(|error| {
        format!(
            "failed moving {} to rollback backup {}: {error}",
            target.display(),
            backup.display()
        )
    })?;
    fs::rename(replacement, target).map_err(|error| {
        format!(
            "failed installing replacement {} at {}: {error}",
            replacement.display(),
            target.display()
        )
    })
}

#[cfg(windows)]
fn atomic_restore_backup(target: &Path, backup: &Path) -> Result<(), String> {
    windows_replace_file(target, backup, None).map_err(|error| {
        format!(
            "failed atomically restoring {} from {}: {error}",
            target.display(),
            backup.display()
        )
    })
}

#[cfg(not(windows))]
fn atomic_restore_backup(target: &Path, backup: &Path) -> Result<(), String> {
    fs::rename(backup, target).map_err(|error| {
        format!(
            "failed atomically restoring {} from {}: {error}",
            target.display(),
            backup.display()
        )
    })
}

#[cfg(windows)]
fn windows_replace_file(
    target: &Path,
    replacement: &Path,
    backup: Option<&Path>,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let target = wide_path(target);
    let replacement = wide_path(replacement);
    let backup = backup.map(wide_path);
    let backup_ptr = backup
        .as_ref()
        .map_or(std::ptr::null(), |path| path.as_ptr());
    // SAFETY: all strings are live, NUL-terminated UTF-16 buffers for the duration of the call;
    // the optional backup pointer is either null or points to an equally live buffer.
    let replaced = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            backup_ptr,
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn journal_for_plan(plan: &ReplacementPlan) -> Result<ReplacementJournal, String> {
    let entries = plan
        .files
        .iter()
        .map(|file| {
            Ok(ReplacementJournalEntry {
                relative: file.relative.clone(),
                temporary: file
                    .temporary
                    .strip_prefix(&plan.target_dir)
                    .map_err(|error| error.to_string())?
                    .to_path_buf(),
                backup: file
                    .backup
                    .strip_prefix(&plan.target_dir)
                    .map_err(|error| error.to_string())?
                    .to_path_buf(),
                target_existed: file.target_existed,
                original_sha256: file.original_sha256.clone(),
                replacement_sha256: file.expected_sha256.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ReplacementJournal {
        schema: JOURNAL_SCHEMA.to_owned(),
        entries,
    })
}

fn write_journal_header(path: &Path, journal: &ReplacementJournal) -> Result<(), String> {
    reject_reparse_parent(
        path.parent()
            .ok_or_else(|| "replacement journal has no parent".to_owned())?,
        path,
    )?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "failed creating replacement journal {}: {error}",
                path.display()
            )
        })?;
    serde_json::to_writer(&mut file, journal)
        .map_err(|error| format!("failed serializing replacement journal: {error}"))?;
    file.write_all(b"\n")
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            format!(
                "failed flushing replacement journal {}: {error}",
                path.display()
            )
        })
}

fn append_journal_commit(path: &Path) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "failed opening replacement journal {}: {error}",
                path.display()
            )
        })?;
    serde_json::to_writer(&mut file, &JournalCommit { committed: true })
        .map_err(|error| format!("failed serializing replacement commit: {error}"))?;
    file.write_all(b"\n")
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            format!(
                "failed flushing replacement commit {}: {error}",
                path.display()
            )
        })
}

fn recover_pending_update(target_dir: &Path) -> Result<(), String> {
    let journal_path = target_dir.join(JOURNAL_FILE);
    match fs::symlink_metadata(&journal_path) {
        Ok(metadata) if metadata_is_reparse_or_symlink(&metadata) => {
            return Err(format!(
                "refusing replacement journal link or reparse point {}",
                journal_path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed inspecting replacement journal: {error}")),
    }
    let contents = match fs::read_to_string(&journal_path) {
        Ok(contents) => contents,
        Err(error) => return Err(format!("failed reading replacement journal: {error}")),
    };
    let mut lines = contents.lines();
    let header = lines
        .next()
        .ok_or_else(|| "replacement journal is empty".to_owned())?;
    let journal: ReplacementJournal = serde_json::from_str(header)
        .map_err(|error| format!("replacement journal is corrupt: {error}"))?;
    validate_journal(&journal)?;
    let committed = lines.any(|line| {
        serde_json::from_str::<JournalCommit>(line)
            .map(|commit| commit.committed)
            .unwrap_or(false)
    });
    if committed {
        cleanup_committed_update(target_dir, &journal, &journal_path)
    } else {
        rollback_uncommitted_update(target_dir, &journal)?;
        remove_file_if_exists(&journal_path)
    }
}

fn recover_pending_update_with_retry(target_dir: &Path) -> Result<(), String> {
    const ATTEMPTS: usize = 8;
    let mut delay = std::time::Duration::from_millis(25);
    for attempt in 1..=ATTEMPTS {
        match recover_pending_update(target_dir) {
            Ok(()) => return Ok(()),
            Err(error) if attempt == ATTEMPTS => return Err(error),
            Err(_) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(400));
            }
        }
    }
    unreachable!("the bounded recovery loop always returns")
}

fn validate_journal(journal: &ReplacementJournal) -> Result<(), String> {
    if journal.schema != JOURNAL_SCHEMA {
        return Err(format!(
            "unsupported replacement journal schema {:?}",
            journal.schema
        ));
    }
    for entry in &journal.entries {
        if !relative_path_is_safe(&entry.relative)
            || !relative_path_is_safe(&entry.temporary)
            || !relative_path_is_safe(&entry.backup)
        {
            return Err("replacement journal contains an unsafe path".to_owned());
        }
        if entry.temporary.parent() != entry.relative.parent()
            || entry.backup.parent() != entry.relative.parent()
            || entry.relative == entry.temporary
            || entry.relative == entry.backup
            || entry.temporary == entry.backup
        {
            return Err("replacement journal artifact is not beside its target".to_owned());
        }
        if entry.target_existed != entry.original_sha256.is_some() {
            return Err("replacement journal is missing the original file digest".to_owned());
        }
        for digest in [
            entry.original_sha256.as_deref(),
            entry.replacement_sha256.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_sha256_hex(digest)
                .map_err(|_| "replacement journal contains an invalid digest".to_owned())?;
        }
    }
    Ok(())
}

fn rollback_journal_entry(
    target_dir: &Path,
    entry: &ReplacementJournalEntry,
) -> Result<(), String> {
    let target = target_dir.join(&entry.relative);
    let temporary = target_dir.join(&entry.temporary);
    let backup = target_dir.join(&entry.backup);
    let target_digest = regular_file_digest_if_present(&target, "rollback target")?;
    let temporary_digest = regular_file_digest_if_present(&temporary, "prepared replacement")?;
    let backup_digest = regular_file_digest_if_present(&backup, "rollback backup")?;

    if let Some(actual) = temporary_digest.as_deref() {
        let expected = entry.replacement_sha256.as_deref().ok_or_else(|| {
            format!(
                "unexpected prepared replacement exists for removed file {}",
                entry.relative.display()
            )
        })?;
        ensure_digest_matches(actual, expected, &temporary, "prepared replacement")?;
    }

    if entry.target_existed {
        let original = entry.original_sha256.as_deref().ok_or_else(|| {
            format!(
                "rollback journal has no original digest for {}",
                entry.relative.display()
            )
        })?;
        if let Some(actual) = backup_digest.as_deref() {
            ensure_digest_matches(actual, original, &backup, "rollback backup")?;
            if let Some(actual) = target_digest.as_deref()
                && actual != original
                && entry.replacement_sha256.as_deref() != Some(actual)
            {
                return Err(format!(
                    "rollback target {} has an unrecognized digest; recovery journal retained",
                    target.display()
                ));
            }
            if target_digest.is_some() {
                atomic_restore_backup(&target, &backup)?;
            } else {
                fs::rename(&backup, &target).map_err(|error| {
                    format!(
                        "failed restoring missing target {} from {}: {error}",
                        target.display(),
                        backup.display()
                    )
                })?;
            }
        } else {
            match target_digest.as_deref() {
                Some(actual) if actual == original => {}
                Some(actual) if entry.replacement_sha256.as_deref() == Some(actual) => {
                    return Err(format!(
                        "rollback backup is missing while {} contains the replacement; recovery journal retained",
                        target.display()
                    ));
                }
                Some(_) => {
                    return Err(format!(
                        "rollback target {} has an unrecognized digest and no backup; recovery journal retained",
                        target.display()
                    ));
                }
                None => {
                    return Err(format!(
                        "rollback target and backup are both missing for {}; recovery journal retained",
                        entry.relative.display()
                    ));
                }
            }
        }
    } else {
        if backup_digest.is_some() {
            return Err(format!(
                "unexpected rollback backup exists for newly installed file {}; recovery journal retained",
                entry.relative.display()
            ));
        }
        if let Some(actual) = target_digest.as_deref() {
            let expected = entry.replacement_sha256.as_deref().ok_or_else(|| {
                format!(
                    "unexpected target appeared during rollback for {}; recovery journal retained",
                    entry.relative.display()
                )
            })?;
            ensure_digest_matches(actual, expected, &target, "installed target")?;
            remove_file_if_exists(&target)?;
        }
    }

    if temporary_digest.is_some() {
        remove_file_if_exists(&temporary)?;
    }
    Ok(())
}

fn validate_rolled_back_entry(
    target_dir: &Path,
    entry: &ReplacementJournalEntry,
) -> Result<(), String> {
    let target = target_dir.join(&entry.relative);
    match entry.original_sha256.as_deref() {
        Some(original) => require_file_digest(&target, original, "rolled-back target")?,
        None => require_file_absent(&target, "rolled-back target")?,
    }
    require_file_absent(
        &target_dir.join(&entry.temporary),
        "rolled-back prepared replacement",
    )?;
    require_file_absent(&target_dir.join(&entry.backup), "rolled-back backup")
}

fn validate_committed_entry(
    target_dir: &Path,
    entry: &ReplacementJournalEntry,
) -> Result<(), String> {
    let target = target_dir.join(&entry.relative);
    match entry.replacement_sha256.as_deref() {
        Some(expected) => require_file_digest(&target, expected, "committed target")?,
        None => require_file_absent(&target, "committed removed target")?,
    }

    let temporary = target_dir.join(&entry.temporary);
    if let Some(actual) = regular_file_digest_if_present(&temporary, "prepared replacement")? {
        let expected = entry.replacement_sha256.as_deref().ok_or_else(|| {
            format!(
                "unexpected prepared replacement remains for removed file {}",
                entry.relative.display()
            )
        })?;
        ensure_digest_matches(&actual, expected, &temporary, "prepared replacement")?;
    }

    let backup = target_dir.join(&entry.backup);
    if let Some(actual) = regular_file_digest_if_present(&backup, "rollback backup")? {
        let original = entry.original_sha256.as_deref().ok_or_else(|| {
            format!(
                "unexpected rollback backup remains for new file {}",
                entry.relative.display()
            )
        })?;
        ensure_digest_matches(&actual, original, &backup, "rollback backup")?;
    }
    Ok(())
}

fn require_file_digest(path: &Path, expected: &str, role: &str) -> Result<(), String> {
    let actual = regular_file_digest_if_present(path, role)?
        .ok_or_else(|| format!("{role} is missing: {}", path.display()))?;
    ensure_digest_matches(&actual, expected, path, role)
}

fn require_file_absent(path: &Path, role: &str) -> Result<(), String> {
    if regular_file_digest_if_present(path, role)?.is_some() {
        Err(format!("{role} unexpectedly exists: {}", path.display()))
    } else {
        Ok(())
    }
}

fn ensure_digest_matches(
    actual: &str,
    expected: &str,
    path: &Path,
    role: &str,
) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{role} digest mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ))
    }
}

fn regular_file_digest_if_present(path: &Path, role: &str) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_reparse_or_symlink(&metadata) => Err(format!(
            "{role} is a link or reparse point: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => sha256_file(path).map(Some),
        Ok(_) => Err(format!("{role} is not a regular file: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed inspecting {role} {}: {error}",
            path.display()
        )),
    }
}

fn rollback_uncommitted_update(
    target_dir: &Path,
    journal: &ReplacementJournal,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for entry in journal.entries.iter().rev() {
        if let Err(error) = rollback_journal_entry(target_dir, entry) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        for entry in &journal.entries {
            if let Err(error) = validate_rolled_back_entry(target_dir, entry) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn cleanup_committed_update(
    target_dir: &Path,
    journal: &ReplacementJournal,
    journal_path: &Path,
) -> Result<(), String> {
    for entry in &journal.entries {
        validate_committed_entry(target_dir, entry)?;
    }
    for entry in &journal.entries {
        remove_file_if_exists(&target_dir.join(&entry.temporary))?;
        remove_file_if_exists(&target_dir.join(&entry.backup))?;
    }
    remove_file_if_exists(journal_path)
}

fn create_relative_directories_without_reparse(root: &Path, relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    if !relative_path_is_safe(relative) {
        return Err(format!("unsafe directory path {}", relative.display()));
    }
    let mut current = root.to_path_buf();
    ensure_directory_is_not_reparse_point(&current)?;
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("unsafe directory path {}", relative.display()));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata_is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                    return Err(format!(
                        "update path component is not a regular directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "failed creating update directory {}: {error}",
                        current.display()
                    )
                })?;
                ensure_directory_is_not_reparse_point(&current)?;
            }
            Err(error) => return Err(format!("failed inspecting {}: {error}", current.display())),
        }
    }
    Ok(())
}

fn reject_reparse_parent(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("path escapes update root: {}", path.display()))?;
    if let Some(parent) = relative.parent() {
        let mut current = root.to_path_buf();
        ensure_directory_is_not_reparse_point(&current)?;
        for component in parent.components() {
            let Component::Normal(component) = component else {
                return Err(format!("unsafe update path {}", path.display()));
            };
            current.push(component);
            ensure_directory_is_not_reparse_point(&current)?;
        }
    }
    Ok(())
}

fn reject_reparse_path(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed inspecting {}: {error}", path.display()))?;
    if metadata_is_reparse_or_symlink(&metadata) {
        Err(format!(
            "refusing link or reparse-point path {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn ensure_directory_is_not_reparse_point(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed inspecting directory {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata_is_reparse_or_symlink(&metadata) {
        Err(format!(
            "refusing link or reparse-point directory {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn metadata_is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed removing {}: {error}", path.display())),
    }
}

fn remove_directory_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed removing {}: {error}", path.display())),
    }
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

    fn test_root(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("sorotte-updater-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_relative(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, bytes).unwrap();
        path
    }

    fn write_v2_install_source(root: &Path, files: &[(&str, &[u8])]) {
        let manifest = InstallManifest {
            schema: INSTALL_MANIFEST_SCHEMA.to_owned(),
            app: "sorotte-gui".to_owned(),
            version: "0.2.4".to_owned(),
            target: INSTALL_TARGET.to_owned(),
            files: files
                .iter()
                .map(|(path, contents)| {
                    write_relative(root, path, contents);
                    InstallFile {
                        path: (*path).to_owned(),
                        sha256: sha256_bytes(contents),
                    }
                })
                .collect(),
        };
        fs::write(
            root.join(INSTALL_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn test_plan(root: &Path, replacements: &[(&str, Option<&[u8]>)]) -> ReplacementPlan {
        let target_dir = root.join("target");
        let source_dir = root.join("source");
        fs::create_dir_all(&target_dir).unwrap();
        fs::create_dir_all(&source_dir).unwrap();
        let files = replacements
            .iter()
            .map(|(relative, contents)| {
                let (source, expected) = match contents {
                    Some(contents) => {
                        let source = write_relative(&source_dir, relative, contents);
                        (Some(source), Some(sha256_bytes(contents)))
                    }
                    None => (None, None),
                };
                replacement_file(
                    &target_dir,
                    PathBuf::from(relative),
                    source,
                    expected,
                    "test-transaction",
                )
                .unwrap()
            })
            .collect();
        ReplacementPlan {
            target_exe_path: target_dir.join(GUI_EXE),
            journal_path: target_dir.join(JOURNAL_FILE),
            log_path: root.join("update.log"),
            restart: false,
            target_dir,
            files,
        }
    }

    #[cfg(windows)]
    fn prepare_test_plan(plan: &ReplacementPlan) {
        for file in &plan.files {
            if let Some(source) = file.source.as_ref() {
                fs::copy(source, &file.temporary).unwrap();
            }
        }
    }

    #[test]
    fn parse_args_requires_authenticated_package() {
        let error = parse_args(["--pid".to_owned(), "123".to_owned()])
            .expect_err("missing package arguments should fail");
        assert!(error.contains("--package-sha256"));
    }

    #[test]
    fn parse_args_accepts_package_digest_and_restart() {
        let args = parse_args([
            "--pid".to_owned(),
            "123".to_owned(),
            "--package".to_owned(),
            "update.zip".to_owned(),
            "--package-sha256".to_owned(),
            "a".repeat(64),
            "--target-dir".to_owned(),
            "target".to_owned(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--log".to_owned(),
            "update.log".to_owned(),
            "--restart".to_owned(),
        ])
        .unwrap();

        assert!(matches!(
            args.input,
            Some(UpdateInput::Package { ref package, .. }) if package == Path::new("update.zip")
        ));
        assert!(args.restart);
    }

    #[test]
    fn parse_args_accepts_exact_legacy_source_and_backup_pair() {
        let args = parse_args([
            "--pid".to_owned(),
            "123".to_owned(),
            "--source-dir".to_owned(),
            "stage/extracted".to_owned(),
            "--target-dir".to_owned(),
            "target".to_owned(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--backup-dir".to_owned(),
            "stage/backup".to_owned(),
            "--log".to_owned(),
            "update.log".to_owned(),
        ])
        .unwrap();

        assert!(matches!(
            args.input,
            Some(UpdateInput::LegacySource { ref source_dir, .. })
                if source_dir == Path::new("stage/extracted")
        ));
    }

    #[test]
    fn parse_args_accepts_recovery_only_reentry() {
        let args = parse_args([
            "--recover".to_owned(),
            "--pid".to_owned(),
            "123".to_owned(),
            "--target-dir".to_owned(),
            "target".to_owned(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--log".to_owned(),
            "recovery.log".to_owned(),
            "--restart".to_owned(),
        ])
        .unwrap();

        assert_eq!(args.input, None);
        assert!(args.restart);
    }

    #[test]
    fn elevated_execution_is_rejected_before_all_updater_modes() {
        let log_path = test_root("elevated-rejection")
            .join("update.log")
            .display()
            .to_string();
        let argument_sets = [
            vec![
                "--pid".to_owned(),
                "123".to_owned(),
                "--package".to_owned(),
                "update.zip".to_owned(),
                "--package-sha256".to_owned(),
                "a".repeat(64),
                "--target-dir".to_owned(),
                "target".to_owned(),
                "--target-exe".to_owned(),
                GUI_EXE.to_owned(),
                "--log".to_owned(),
                log_path.clone(),
            ],
            vec![
                "--pid".to_owned(),
                "123".to_owned(),
                "--source-dir".to_owned(),
                "source".to_owned(),
                "--target-dir".to_owned(),
                "target".to_owned(),
                "--target-exe".to_owned(),
                GUI_EXE.to_owned(),
                "--backup-dir".to_owned(),
                "backup".to_owned(),
                "--log".to_owned(),
                log_path.clone(),
            ],
            vec![
                "--recover".to_owned(),
                "--pid".to_owned(),
                "123".to_owned(),
                "--target-dir".to_owned(),
                "target".to_owned(),
                "--target-exe".to_owned(),
                GUI_EXE.to_owned(),
                "--log".to_owned(),
                log_path,
            ],
        ];

        for arguments in argument_sets {
            let error =
                run_update_with_elevation_check(parse_args(arguments).unwrap(), || Ok(true))
                    .expect_err("every updater protocol must fail closed when already elevated");
            assert!(error.contains("elevated updater process"));
        }
    }

    #[test]
    fn legacy_old_gui_invocation_bootstraps_v2_source_transactionally() {
        let root = test_root("legacy-bootstrap");
        let source = root.join("stage").join("extracted");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        write_v2_install_source(
            &source,
            &[(GUI_EXE, b"new-gui"), (UPDATER_EXE, b"new-updater")],
        );
        write_relative(&target, GUI_EXE, b"old-gui");
        write_relative(&target, UPDATER_EXE, b"old-updater");
        let old_manifest = InstallManifest {
            schema: String::new(),
            app: "sorotte-gui".to_owned(),
            version: "0.2.3".to_owned(),
            target: INSTALL_TARGET.to_owned(),
            files: Vec::new(),
        };
        fs::write(
            target.join(INSTALL_MANIFEST),
            serde_json::to_vec(&old_manifest).unwrap(),
        )
        .unwrap();
        let args = parse_args([
            "--pid".to_owned(),
            u32::MAX.to_string(),
            "--source-dir".to_owned(),
            source.display().to_string(),
            "--target-dir".to_owned(),
            target.display().to_string(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--backup-dir".to_owned(),
            root.join("stage").join("backup").display().to_string(),
            "--log".to_owned(),
            root.join("update.log").display().to_string(),
        ])
        .unwrap();

        run_update_with_checks(
            args,
            || Ok(false),
            |_| Ok(UpdaterExecutionLocation::DetachedHelper),
        )
        .expect("the exact old-GUI invocation should bootstrap the v2 update");

        assert_eq!(fs::read(target.join(GUI_EXE)).unwrap(), b"new-gui");
        assert_eq!(fs::read(target.join(UPDATER_EXE)).unwrap(), b"new-updater");
        let installed = read_install_manifest(&target.join(INSTALL_MANIFEST)).unwrap();
        assert_eq!(installed.schema, INSTALL_MANIFEST_SCHEMA);
        assert!(!target.join(JOURNAL_FILE).exists());
    }

    #[test]
    fn verified_package_snapshot_rejects_substitution_and_is_used_immutably() {
        let root = test_root("package-substitution");
        let package = write_relative(&root, "update.zip", b"authenticated package");
        let expected = sha256_bytes(b"authenticated package");

        let verified = read_verified_package_bytes(&package, &expected).unwrap();
        fs::write(&package, b"substituted package").unwrap();

        assert_eq!(verified, b"authenticated package");
        let error = read_verified_package_bytes(&package, &expected)
            .expect_err("a substituted package must fail digest verification");
        assert!(error.contains("SHA-256 mismatch"));
    }

    #[test]
    fn extracted_manifest_revalidates_every_payload_digest_and_exact_file_set() {
        let root = test_root("payload-manifest");
        write_relative(&root, GUI_EXE, b"gui");
        write_relative(&root, UPDATER_EXE, b"updater");
        let manifest = InstallManifest {
            schema: INSTALL_MANIFEST_SCHEMA.to_owned(),
            app: "sorotte-gui".to_owned(),
            version: "0.2.4".to_owned(),
            target: INSTALL_TARGET.to_owned(),
            files: vec![
                InstallFile {
                    path: GUI_EXE.to_owned(),
                    sha256: sha256_bytes(b"gui"),
                },
                InstallFile {
                    path: UPDATER_EXE.to_owned(),
                    sha256: sha256_bytes(b"updater"),
                },
            ],
        };
        fs::write(
            root.join(INSTALL_MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        validate_extracted_package(&root, &manifest).unwrap();
        fs::write(root.join(GUI_EXE), b"substituted gui").unwrap();
        let error = validate_extracted_package(&root, &manifest)
            .expect_err("post-extraction payload substitution must fail");
        assert!(error.contains("payload digest mismatch"));
    }

    #[test]
    fn failure_on_nth_replacement_rolls_back_every_prior_file() {
        let root = test_root("nth-replacement");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        write_relative(&target, "b.txt", b"old-b");
        let plan = test_plan(
            &root,
            &[("a.txt", Some(b"new-a")), ("b.txt", Some(b"new-b"))],
        );

        let error = apply_replacement_plan_with_hook(&plan, |progress| {
            if progress == ApplyProgress::BeforeReplace(2) {
                Err("injected second replacement failure".to_owned())
            } else {
                Ok(())
            }
        })
        .expect_err("the injected replacement failure must abort the update");

        assert!(error.contains("all changed files were rolled back"));
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"old-a");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"old-b");
        assert!(!plan.journal_path.exists());
    }

    #[test]
    fn preparation_failure_simulating_disk_exhaustion_leaves_install_unchanged() {
        let root = test_root("prepare-exhaustion");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        write_relative(&target, "b.txt", b"old-b");
        let plan = test_plan(
            &root,
            &[("a.txt", Some(b"new-a")), ("b.txt", Some(b"new-b"))],
        );

        let error = apply_replacement_plan_with_hook(&plan, |progress| {
            if progress == ApplyProgress::BeforePrepare(2) {
                Err("injected disk exhaustion".to_owned())
            } else {
                Ok(())
            }
        })
        .expect_err("the injected preparation failure must abort the update");

        assert!(error.contains("injected disk exhaustion"));
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"old-a");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"old-b");
        assert!(!plan.journal_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_atomic_replacement_keeps_executables_invokable_and_recovers() {
        let root = test_root("executable-journal-recovery");
        let target = root.join("target");
        let system_root = env::var_os("SystemRoot").expect("Windows system root should be set");
        let old_executable = fs::read(PathBuf::from(&system_root).join("System32/cmd.exe"))
            .expect("cmd.exe should be readable");
        let new_executable = fs::read(PathBuf::from(system_root).join("System32/where.exe"))
            .expect("where.exe should be readable");
        write_relative(&target, GUI_EXE, &old_executable);
        write_relative(&target, UPDATER_EXE, &old_executable);
        let plan = test_plan(
            &root,
            &[
                (GUI_EXE, Some(new_executable.as_slice())),
                (UPDATER_EXE, Some(new_executable.as_slice())),
            ],
        );
        let journal = journal_for_plan(&plan).unwrap();
        write_journal_header(&plan.journal_path, &journal).unwrap();
        prepare_test_plan(&plan);
        let updater = plan
            .files
            .iter()
            .find(|file| file.relative == Path::new(UPDATER_EXE))
            .unwrap();
        install_prepared_replacement(updater).unwrap();

        assert!(target.join(GUI_EXE).is_file());
        assert!(target.join(UPDATER_EXE).is_file());
        Command::new(target.join(UPDATER_EXE))
            .arg("/?")
            .status()
            .expect("atomically installed updater should remain invokable");

        let recovery_args = parse_args([
            "--recover".to_owned(),
            "--pid".to_owned(),
            u32::MAX.to_string(),
            "--target-dir".to_owned(),
            target.display().to_string(),
            "--target-exe".to_owned(),
            GUI_EXE.to_owned(),
            "--log".to_owned(),
            root.join("recovery.log").display().to_string(),
        ])
        .unwrap();
        run_update_with_checks(
            recovery_args,
            || Ok(false),
            |_| Ok(UpdaterExecutionLocation::DetachedHelper),
        )
        .expect("the recovery-only updater re-entry path should roll back the interruption");

        assert_eq!(fs::read(target.join(GUI_EXE)).unwrap(), old_executable);
        assert_eq!(fs::read(target.join(UPDATER_EXE)).unwrap(), old_executable);
        Command::new(target.join(UPDATER_EXE))
            .args(["/C", "exit /b 0"])
            .status()
            .expect("recovered updater should remain invokable");
        assert!(!plan.journal_path.exists());
        assert!(plan.files.iter().all(|file| !file.backup.exists()));
    }

    #[test]
    fn missing_backup_with_replacement_target_is_ambiguous_and_retains_journal() {
        let root = test_root("missing-backup-ambiguity");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        let plan = test_plan(&root, &[("a.txt", Some(b"new-a"))]);
        let journal = journal_for_plan(&plan).unwrap();
        write_journal_header(&plan.journal_path, &journal).unwrap();
        fs::write(&plan.files[0].target, b"new-a").unwrap();

        let error = recover_pending_update(&plan.target_dir)
            .expect_err("a replacement without its original backup is ambiguous");

        assert!(error.contains("rollback backup is missing"));
        assert_eq!(fs::read(&plan.files[0].target).unwrap(), b"new-a");
        assert!(plan.journal_path.is_file());
    }

    #[test]
    fn missing_original_target_and_backup_retains_recovery_journal() {
        let root = test_root("missing-original-and-backup");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        let plan = test_plan(&root, &[("a.txt", Some(b"new-a"))]);
        let journal = journal_for_plan(&plan).unwrap();
        write_journal_header(&plan.journal_path, &journal).unwrap();
        fs::remove_file(&plan.files[0].target).unwrap();

        let error = recover_pending_update(&plan.target_dir)
            .expect_err("missing original and backup cannot be silently accepted");

        assert!(error.contains("target and backup are both missing"));
        assert!(plan.journal_path.is_file());
    }

    #[test]
    fn committed_cleanup_validates_targets_before_deleting_journal() {
        let root = test_root("committed-validation");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        let plan = test_plan(&root, &[("a.txt", Some(b"new-a"))]);
        let journal = journal_for_plan(&plan).unwrap();
        write_journal_header(&plan.journal_path, &journal).unwrap();
        append_journal_commit(&plan.journal_path).unwrap();

        let error = recover_pending_update(&plan.target_dir)
            .expect_err("an unapplied target must invalidate committed cleanup");

        assert!(error.contains("committed target digest mismatch"));
        assert!(plan.journal_path.is_file());
    }

    #[test]
    fn committed_plan_removes_obsolete_files() {
        let root = test_root("obsolete-removal");
        let target = root.join("target");
        write_relative(&target, "current.txt", b"old-current");
        write_relative(&target, "obsolete.txt", b"obsolete");
        let plan = test_plan(
            &root,
            &[
                ("current.txt", Some(b"new-current")),
                ("obsolete.txt", None),
            ],
        );

        apply_replacement_plan(&plan).unwrap();

        assert_eq!(
            fs::read(target.join("current.txt")).unwrap(),
            b"new-current"
        );
        assert!(!target.join("obsolete.txt").exists());
        assert!(!plan.journal_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_target_failure_rolls_back_prior_replacements() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = test_root("locked-target");
        let target = root.join("target");
        write_relative(&target, "a.txt", b"old-a");
        write_relative(&target, "b.txt", b"old-b");
        let plan = test_plan(
            &root,
            &[("a.txt", Some(b"new-a")), ("b.txt", Some(b"new-b"))],
        );
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("b.txt"))
            .unwrap();

        let error = apply_replacement_plan(&plan)
            .expect_err("a non-delete-shared target must abort replacement");

        assert!(error.contains("rollback was incomplete"));
        assert!(plan.journal_path.is_file());
        let release_lock = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(75));
            drop(lock);
        });
        recover_pending_update_with_retry(&plan.target_dir)
            .expect("recovery retry should finish after the transient file lock is released");
        release_lock.join().unwrap();
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"old-a");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"old-b");
        assert!(!plan.journal_path.exists());
    }

    #[test]
    fn reparse_or_symlink_package_paths_are_rejected() {
        let root = test_root("reparse");
        let real = root.join("real");
        let link = root.join("linked");
        fs::create_dir(&real).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        {
            let status = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&real)
                .status()
                .unwrap();
            if !status.success() {
                return;
            }
        }

        let error = ensure_directory_is_not_reparse_point(&link)
            .expect_err("directory links and reparse points must be rejected");
        assert!(error.contains("reparse-point"));
    }

    #[test]
    fn relative_path_rejects_parent_and_absolute_components() {
        assert!(relative_path_is_safe(Path::new("resources/script.lua")));
        assert!(!relative_path_is_safe(Path::new("../sorotte-gui.exe")));
        assert!(!relative_path_is_safe(Path::new("/sorotte-gui.exe")));
    }
}
