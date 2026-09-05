//! A media tool and both pipes have one owner and one execution/drain deadline.
//!
//! No reader thread can outlive the operation. A read is attempted only when the
//! pipe is ready, and each turn consumes at most one chunk from each pipe so that
//! endless output cannot starve cancellation or the other pipe. Owned process
//! groups/jobs are terminated and observed before returning an error. Cleanup
//! has a separate, short bound; failure to observe cleanup is an explicit error.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::{MediaFingerprintError, MediaToolProcessIoMetrics, MediaToolStreamingOutput};
use crate::tuning::MEDIA_TOOL_POLL_INTERVAL;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::{OwnedTool, Pipe};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::{OwnedTool, Pipe};

pub(super) const PROBE_STDOUT_LIMIT: usize = 64 * 1024;
const STDERR_TAIL_LIMIT: usize = 16 * 1024;
const STDERR_EMISSION_LIMIT: usize = 1024 * 1024;
const PIPE_CHUNK_BYTES: usize = 16 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(super) struct Deadline {
    at: Instant,
    timeout: Duration,
}

impl Deadline {
    pub(super) fn after(timeout: Duration) -> Self {
        Self {
            at: Instant::now() + timeout,
            timeout,
        }
    }

    pub(super) fn capped(self, timeout: Duration) -> Self {
        let cap = Self::after(timeout);
        if self.at <= cap.at { self } else { cap }
    }

    pub(super) fn check(
        self,
        tool: &'static str,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<(), MediaFingerprintError> {
        check_cancelled(tool, cancel_flag)?;
        if Instant::now() >= self.at {
            return Err(MediaFingerprintError::TimedOut {
                tool,
                timeout_seconds: self.timeout.as_secs().max(1),
            });
        }
        Ok(())
    }

    fn pause(self) {
        thread::sleep(
            MEDIA_TOOL_POLL_INTERVAL.min(self.at.saturating_duration_since(Instant::now())),
        );
    }
}

pub(super) fn check_cancelled(
    tool: &'static str,
    cancel_flag: Option<&AtomicBool>,
) -> Result<(), MediaFingerprintError> {
    if cancel_flag.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(MediaFingerprintError::Cancelled { tool })
    } else {
        Ok(())
    }
}

pub(super) fn command(executable: &Path, args: impl IntoIterator<Item = OsString>) -> Command {
    let mut command = Command::new(executable);
    command.args(args);
    command
}

pub(super) fn run_output(
    tool: &'static str,
    command: Command,
    cancel_flag: Option<&AtomicBool>,
    deadline: Deadline,
    stdout_limit: usize,
) -> Result<(Output, MediaToolStreamingOutput), MediaFingerprintError> {
    let mut stdout = Vec::new();
    let (status, stderr, streaming) = run_streaming(
        tool,
        command,
        cancel_flag,
        deadline,
        stdout_limit,
        |chunk| {
            stdout.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    Ok((
        Output {
            status,
            stdout,
            stderr,
        },
        streaming,
    ))
}

fn tool_error(tool: &'static str, error: impl std::fmt::Display) -> MediaFingerprintError {
    MediaFingerprintError::ToolFailed {
        tool,
        status: None,
        stderr: error.to_string(),
    }
}

fn push_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    let keep = chunk.len().min(STDERR_TAIL_LIMIT);
    let remove = (tail.len() + keep).saturating_sub(STDERR_TAIL_LIMIT);
    tail.drain(..remove);
    tail.extend_from_slice(&chunk[chunk.len() - keep..]);
}

fn output_limit_error(
    tool: &'static str,
    pipe: &str,
    limit: usize,
    stderr: &[u8],
) -> MediaFingerprintError {
    tool_error(
        tool,
        format!(
            "{pipe} exceeded its {limit}-byte limit; stderr tail: {}",
            String::from_utf8_lossy(stderr).trim(),
        ),
    )
}

/// Returns `None` while no data is ready, `Some(0)` at EOF, or one ready chunk.
fn read_ready(pipe: &mut Pipe, buffer: &mut [u8]) -> io::Result<Option<usize>> {
    if !pipe.ready()? {
        return Ok(None);
    }
    match pipe.read(buffer) {
        Ok(count) => Ok(Some(count)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn run_streaming(
    tool: &'static str,
    mut command: Command,
    cancel_flag: Option<&AtomicBool>,
    deadline: Deadline,
    stdout_limit: usize,
    on_stdout_chunk: impl FnMut(&[u8]) -> Result<(), MediaFingerprintError>,
) -> Result<(ExitStatus, Vec<u8>, MediaToolStreamingOutput), MediaFingerprintError> {
    deadline.check(tool, cancel_flag)?;
    let started_at = Instant::now();
    // The platform launcher checks once more immediately before creating a child.
    let (child, stdout, stderr) =
        OwnedTool::spawn(tool, &mut command, || deadline.check(tool, cancel_flag))?;
    run_started(
        tool,
        StartedTool {
            child,
            stdout,
            stderr,
            started_at,
        },
        cancel_flag,
        deadline,
        stdout_limit,
        on_stdout_chunk,
    )
}

struct StartedTool {
    child: OwnedTool,
    stdout: Pipe,
    stderr: Pipe,
    started_at: Instant,
}

fn run_started(
    tool: &'static str,
    started: StartedTool,
    cancel_flag: Option<&AtomicBool>,
    deadline: Deadline,
    stdout_limit: usize,
    mut on_stdout_chunk: impl FnMut(&[u8]) -> Result<(), MediaFingerprintError>,
) -> Result<(ExitStatus, Vec<u8>, MediaToolStreamingOutput), MediaFingerprintError> {
    let StartedTool {
        mut child,
        mut stdout,
        mut stderr,
        started_at,
    } = started;
    let mut tail = Vec::new();
    let result = (|| {
        let mut stdout_bytes = 0usize;
        let mut stderr_bytes = 0usize;
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        let mut exit = None;
        let mut exit_millis = 0;
        let mut buffer = [0u8; PIPE_CHUNK_BYTES];
        loop {
            deadline.check(tool, cancel_flag)?;
            let mut progressed = false;
            // Stderr goes first so a simultaneous stdout-limit error retains the
            // latest available diagnostic rather than an empty error message.
            if !stderr_eof
                && let Some(count) = read_ready(&mut stderr, &mut buffer)
                    .map_err(|error| tool_error(tool, format!("reading stderr: {error}")))?
            {
                progressed = count != 0;
                stderr_eof = count == 0;
                push_tail(&mut tail, &buffer[..count]);
                stderr_bytes = stderr_bytes.saturating_add(count);
                if stderr_bytes > STDERR_EMISSION_LIMIT {
                    return Err(output_limit_error(
                        tool,
                        "stderr",
                        STDERR_EMISSION_LIMIT,
                        &tail,
                    ));
                }
            }
            deadline.check(tool, cancel_flag)?;
            if !stdout_eof
                && let Some(count) = read_ready(&mut stdout, &mut buffer)
                    .map_err(|error| tool_error(tool, format!("reading stdout: {error}")))?
            {
                progressed |= count != 0;
                stdout_eof = count == 0;
                stdout_bytes = stdout_bytes.saturating_add(count);
                if stdout_bytes > stdout_limit {
                    return Err(output_limit_error(tool, "stdout", stdout_limit, &tail));
                }
                if count != 0 {
                    on_stdout_chunk(&buffer[..count])?;
                }
            }
            if exit.is_none() {
                exit = child.try_wait().map_err(|error| tool_error(tool, error))?;
                if exit.is_some() {
                    exit_millis = started_at.elapsed().as_millis();
                }
            }
            if let Some(status) = exit
                && stdout_eof
                && stderr_eof
            {
                deadline.check(tool, cancel_flag)?;
                return Ok((
                    status,
                    MediaToolStreamingOutput {
                        stdout_bytes: stdout_bytes as u64,
                        process_io: child.io_counters(),
                        exit_millis,
                    },
                ));
            }
            if !progressed {
                deadline.pause();
            }
        }
    })();

    // Close our pipe handles before termination. Nothing can block waiting for a
    // descendant to release its writer, and there are no detached reader owners.
    drop(stdout);
    drop(stderr);
    if let Err(error) = child.finish(CLEANUP_TIMEOUT) {
        let context = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_else(|| "media tool completed".to_owned());
        return Err(tool_error(
            tool,
            format!("{context}; owned-process cleanup incomplete: {error}"),
        ));
    }
    let (status, streaming) = result?;
    deadline.check(tool, cancel_flag)?;
    Ok((status, tail, streaming))
}

#[cfg(test)]
mod tests;
