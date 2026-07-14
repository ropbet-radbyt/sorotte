use std::{
    env,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use sorotte_player_api::PlayerMediaGeneration;

const YTDL_LIVE_PRINT_TEMPLATE: &str = r#"{"is_live":%(is_live)j}"#;
const MAX_YTDL_LIVE_PROBE_OUTPUT_BYTES: usize = 64 * 1024;
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum YtdlLiveMetadataCapability {
    Unknown,
    ExternalProbeRequired,
    NativeMetadata,
}

impl YtdlLiveMetadataCapability {
    pub(crate) fn from_mpv_version(version: Option<(u64, u64)>) -> Self {
        match version {
            Some((major, minor)) if major > 0 || minor >= 39 => Self::NativeMetadata,
            Some(_) => Self::ExternalProbeRequired,
            None => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum YtdlLiveProbeOutcome {
    IsLive(bool),
    Failed,
    TimedOut,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct YtdlLiveProbeCompletion {
    pub(crate) media_generation: PlayerMediaGeneration,
    pub(crate) target: String,
    pub(crate) outcome: YtdlLiveProbeOutcome,
}

impl std::fmt::Debug for YtdlLiveProbeCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("YtdlLiveProbeCompletion")
            .field("media_generation", &self.media_generation)
            .field("target", &sorotte_secret::REDACTED_SECRET)
            .field("outcome", &self.outcome)
            .finish()
    }
}

pub(crate) struct PendingYtdlLiveProbe {
    pub(crate) media_generation: PlayerMediaGeneration,
    pub(crate) target: String,
    pub(crate) completion_rx: Mutex<Receiver<YtdlLiveProbeCompletion>>,
    pub(crate) cancellation: Arc<AtomicBool>,
}

impl Drop for PendingYtdlLiveProbe {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

pub(crate) fn spawn_ytdl_live_probe(
    executable: Option<PathBuf>,
    path_prefixes: Vec<PathBuf>,
    media_generation: PlayerMediaGeneration,
    target: String,
    execution_target: String,
    timeout: Duration,
) -> PendingYtdlLiveProbe {
    let (completion_tx, completion_rx) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker_target = target.clone();
    let worker_execution_target = execution_target;
    let _ = std::thread::Builder::new()
        .name(format!(
            "sorotte-ytdl-live-probe-{}",
            media_generation.get()
        ))
        .spawn(move || {
            let outcome = run_ytdl_live_probe(
                executable,
                path_prefixes,
                &worker_execution_target,
                timeout,
                &worker_cancellation,
            );
            let _ = completion_tx.send(YtdlLiveProbeCompletion {
                media_generation,
                target: worker_target,
                outcome,
            });
        });

    PendingYtdlLiveProbe {
        media_generation,
        target,
        completion_rx: Mutex::new(completion_rx),
        cancellation,
    }
}

pub(crate) fn youtube_live_probe_execution_target(target: &str) -> Option<&str> {
    let mut target = target.trim();
    if let Some(stripped) = target.strip_prefix("ytdl://") {
        target = stripped;
    }
    let (scheme, remainder) = target.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    let host_and_port = authority.rsplit('@').next().unwrap_or_default();
    let host = host_and_port
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();

    (matches!(
        host.as_str(),
        "youtube.com" | "youtu.be" | "youtube-nocookie.com"
    ) || host.ends_with(".youtube.com")
        || host.ends_with(".youtube-nocookie.com"))
    .then_some(target)
}

fn run_ytdl_live_probe(
    configured_executable: Option<PathBuf>,
    path_prefixes: Vec<PathBuf>,
    target: &str,
    timeout: Duration,
    cancellation: &AtomicBool,
) -> YtdlLiveProbeOutcome {
    let executables = configured_executable.map_or_else(
        || vec![PathBuf::from("yt-dlp"), PathBuf::from("youtube-dl")],
        |executable| vec![executable],
    );

    for executable in executables {
        if cancellation.load(Ordering::Acquire) {
            return YtdlLiveProbeOutcome::Failed;
        }
        let Some(mut command) = build_ytdl_live_probe_command(executable, &path_prefixes, target)
        else {
            return YtdlLiveProbeOutcome::Failed;
        };

        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return YtdlLiveProbeOutcome::Failed,
        };
        return match wait_for_bounded_stdout(child, timeout, cancellation) {
            BoundedChildOutput::Completed {
                success: true,
                stdout,
            } => parse_ytdl_live_probe_output(&stdout)
                .map(YtdlLiveProbeOutcome::IsLive)
                .unwrap_or(YtdlLiveProbeOutcome::Failed),
            BoundedChildOutput::Completed { .. } => YtdlLiveProbeOutcome::Failed,
            BoundedChildOutput::TimedOut => YtdlLiveProbeOutcome::TimedOut,
            BoundedChildOutput::Cancelled | BoundedChildOutput::Failed => {
                YtdlLiveProbeOutcome::Failed
            }
        };
    }

    YtdlLiveProbeOutcome::Failed
}

fn build_ytdl_live_probe_command(
    executable: PathBuf,
    path_prefixes: &[PathBuf],
    target: &str,
) -> Option<Command> {
    let mut command = Command::new(executable);
    command
        .args([
            "--ignore-config",
            "--no-playlist",
            "--skip-download",
            "--no-warnings",
            "--no-progress",
            "--print",
            YTDL_LIVE_PRINT_TEMPLATE,
            "--",
            target,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !path_prefixes.is_empty() {
        let mut probe_path = path_prefixes.to_vec();
        if let Some(existing_path) = env::var_os("PATH") {
            probe_path.extend(env::split_paths(&existing_path));
        }
        let probe_path = env::join_paths(probe_path).ok()?;
        command.env("PATH", probe_path);
    }
    configure_background_child(&mut command);
    Some(command)
}

enum BoundedChildOutput {
    Completed { success: bool, stdout: Vec<u8> },
    TimedOut,
    Cancelled,
    Failed,
}

fn wait_for_bounded_stdout(
    mut child: Child,
    timeout: Duration,
    cancellation: &AtomicBool,
) -> BoundedChildOutput {
    let started = Instant::now();
    loop {
        if cancellation.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return BoundedChildOutput::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let Some(mut stdout) = child.stdout.take() else {
                    return BoundedChildOutput::Failed;
                };
                let mut bytes = Vec::new();
                if stdout
                    .by_ref()
                    .take((MAX_YTDL_LIVE_PROBE_OUTPUT_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .is_err()
                    || bytes.len() > MAX_YTDL_LIVE_PROBE_OUTPUT_BYTES
                {
                    return BoundedChildOutput::Failed;
                }
                return BoundedChildOutput::Completed {
                    success: status.success(),
                    stdout: bytes,
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(CHILD_POLL_INTERVAL.min(timeout));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return BoundedChildOutput::TimedOut;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return BoundedChildOutput::Failed;
            }
        }
    }
}

fn parse_ytdl_live_probe_output(output: &[u8]) -> Option<bool> {
    if output.len() > MAX_YTDL_LIVE_PROBE_OUTPUT_BYTES {
        return None;
    }
    std::str::from_utf8(output)
        .ok()?
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|value| {
            value
                .as_bool()
                .or_else(|| value.get("is_live").and_then(Value::as_bool))
        })
}

#[cfg(windows)]
fn configure_background_child(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_background_child(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    const HELPER_MODE_ENV: &str = "SOROTTE_YTDL_LIVE_PROBE_TEST_HELPER";

    #[test]
    fn parser_accepts_only_bounded_boolean_json() {
        assert_eq!(
            parse_ytdl_live_probe_output(b"{\"is_live\":true}\n"),
            Some(true)
        );
        assert_eq!(parse_ytdl_live_probe_output(b"false\n"), Some(false));
        assert_eq!(
            parse_ytdl_live_probe_output(b"{\"is_live\":\"true\"}\n"),
            None
        );
        assert_eq!(parse_ytdl_live_probe_output(b"not-json\n"), None);
        assert_eq!(
            parse_ytdl_live_probe_output(&vec![b'x'; MAX_YTDL_LIVE_PROBE_OUTPUT_BYTES + 1]),
            None
        );
    }

    #[test]
    fn youtube_scope_excludes_direct_and_signed_media_urls() {
        assert!(
            youtube_live_probe_execution_target("https://www.youtube.com/watch?v=example")
                .is_some()
        );
        assert!(youtube_live_probe_execution_target("https://youtu.be/example").is_some());
        assert!(
            youtube_live_probe_execution_target("ytdl://https://music.youtube.com/watch?v=example")
                .is_some()
        );
        assert!(
            youtube_live_probe_execution_target(
                "https://youtube.com.attacker.invalid/watch?v=example"
            )
            .is_none()
        );
        assert!(
            youtube_live_probe_execution_target(
                "https://rr1---sn.example.googlevideo.com/videoplayback?sig=secret"
            )
            .is_none()
        );
        assert!(
            youtube_live_probe_execution_target(
                "https://plex.invalid/video/:/transcode?token=secret"
            )
            .is_none()
        );
    }

    #[test]
    fn ytdl_force_hook_prefix_is_removed_from_the_probe_invocation() {
        let source_target = " ytdl://https://www.youtube.com/watch?v=example ";
        let execution_target = youtube_live_probe_execution_target(source_target)
            .expect("force-hook target should be recognized");
        let command = build_ytdl_live_probe_command(PathBuf::from("yt-dlp"), &[], execution_target)
            .expect("empty PATH prefixes should always be valid");
        let invocation_target = command
            .get_args()
            .last()
            .and_then(|argument| argument.to_str());

        assert_eq!(execution_target, "https://www.youtube.com/watch?v=example");
        assert_eq!(invocation_target, Some(execution_target));
        assert_ne!(invocation_target, Some(source_target.trim()));
    }

    #[test]
    fn bounded_wait_kills_a_hung_probe_child() {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args([
                "--exact",
                "live_probe::tests::live_probe_timeout_helper",
                "--nocapture",
            ])
            .env(HELPER_MODE_ENV, "hang")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_background_child(&mut command);
        let child = command.spawn().expect("helper child should start");
        let started = Instant::now();
        let cancellation = AtomicBool::new(false);

        assert!(matches!(
            wait_for_bounded_stdout(child, Duration::from_millis(100), &cancellation),
            BoundedChildOutput::TimedOut
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bounded_child_output_reaches_the_strict_live_parser() {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args([
                "--exact",
                "live_probe::tests::live_probe_timeout_helper",
                "--nocapture",
            ])
            .env(HELPER_MODE_ENV, "live")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_background_child(&mut command);
        let child = command.spawn().expect("helper child should start");
        let cancellation = AtomicBool::new(false);

        let BoundedChildOutput::Completed {
            success: true,
            stdout,
        } = wait_for_bounded_stdout(child, Duration::from_secs(2), &cancellation)
        else {
            panic!("helper should complete successfully");
        };
        assert_eq!(parse_ytdl_live_probe_output(&stdout), Some(true));
    }

    #[test]
    fn supersession_cancellation_kills_and_reaps_a_hung_probe_child() {
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args([
                "--exact",
                "live_probe::tests::live_probe_timeout_helper",
                "--nocapture",
            ])
            .env(HELPER_MODE_ENV, "hang")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_background_child(&mut command);
        let child = command.spawn().expect("helper child should start");
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancellation_trigger = Arc::clone(&cancellation);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancellation_trigger.store(true, Ordering::Release);
        });
        let started = Instant::now();

        assert!(matches!(
            wait_for_bounded_stdout(child, Duration::from_secs(5), &cancellation),
            BoundedChildOutput::Cancelled
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn live_probe_timeout_helper() {
        match std::env::var(HELPER_MODE_ENV).as_deref() {
            Ok("hang") => std::thread::sleep(Duration::from_secs(30)),
            Ok("live") => println!(r#"{{"is_live":true}}"#),
            _ => {}
        }
    }

    #[test]
    fn capability_requires_an_explicit_old_version() {
        assert_eq!(
            YtdlLiveMetadataCapability::from_mpv_version(Some((0, 34))),
            YtdlLiveMetadataCapability::ExternalProbeRequired
        );
        assert_eq!(
            YtdlLiveMetadataCapability::from_mpv_version(Some((0, 38))),
            YtdlLiveMetadataCapability::ExternalProbeRequired
        );
        assert_eq!(
            YtdlLiveMetadataCapability::from_mpv_version(Some((0, 39))),
            YtdlLiveMetadataCapability::NativeMetadata
        );
        assert_eq!(
            YtdlLiveMetadataCapability::from_mpv_version(None),
            YtdlLiveMetadataCapability::Unknown
        );
    }

    #[test]
    fn completion_debug_redacts_the_probed_url() {
        let completion = YtdlLiveProbeCompletion {
            media_generation: PlayerMediaGeneration::new(7),
            target: "https://youtube.com/watch?v=secret-token".to_owned(),
            outcome: YtdlLiveProbeOutcome::IsLive(true),
        };

        let debug = format!("{completion:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
    }
}
