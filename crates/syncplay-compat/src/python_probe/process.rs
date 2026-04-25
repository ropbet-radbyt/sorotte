use super::*;

pub(crate) fn run_python_probe_raw(
    extra_args: &[&str],
    stdin_payload: &[u8],
) -> Result<String, InteropError> {
    run_python_probe_raw_with_overrides(extra_args, stdin_payload, None, false, &[], false)
}

pub(crate) fn run_python_probe_raw_with_overrides(
    extra_args: &[&str],
    stdin_payload: &[u8],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
    tls_available: bool,
) -> Result<String, InteropError> {
    let legacy_checkout = ensure_legacy_syncplay_checkout_available()?;

    let probe_script = python_handshake_probe_script_path();
    if !probe_script.is_file() {
        return Err(InteropError::PythonHandshakeProbeMissing(probe_script));
    }

    let python_bin = python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&probe_script)
        .arg(&legacy_checkout)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(template) = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
    {
        command.env("SYNCPLAY_PROBE_MOTD_TEMPLATE", template);
    }
    if persistent_rooms_enabled {
        command.env("SYNCPLAY_PROBE_PERSISTENT_ROOMS", "1");
    }
    if !permanent_rooms.is_empty() {
        command.env("SYNCPLAY_PROBE_PERMANENT_ROOMS", permanent_rooms.join("\n"));
    }
    if tls_available {
        command.env("SYNCPLAY_PROBE_TLS_AVAILABLE", "1");
    }
    for arg in extra_args {
        command.arg(arg);
    }

    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let mut stdin = child.stdin.take().ok_or(InteropError::PythonStdinMissing)?;
    stdin
        .write_all(stdin_payload)
        .map_err(InteropError::PythonStdinWrite)?;
    drop(stdin);

    let output = child.wait_with_output().map_err(InteropError::PythonWait)?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    if !output.status.success() {
        return Err(InteropError::PythonProbeFailed {
            exit_code: output.status.code(),
            stdout: stdout.trim().to_owned(),
            stderr: stderr.trim().to_owned(),
        });
    }

    Ok(stdout)
}

pub(crate) fn first_non_empty_stdout_line(stdout: &str) -> Option<&str> {
    stdout.lines().map(str::trim).find(|line| !line.is_empty())
}

pub(crate) fn python_bin_from_env() -> OsString {
    env::var_os("SYNCPLAY_PYTHON_BIN").unwrap_or_else(|| OsString::from("python"))
}
