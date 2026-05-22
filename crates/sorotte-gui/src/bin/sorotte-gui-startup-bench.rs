#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde::{Deserialize, Serialize};
use sorotte_client_app::app_boundary::{
    persistence::upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    state::StoredClientSettingsMvp,
};

#[cfg(target_os = "windows")]
const MAIN_WINDOW_CONTROLS_CONTAINER_NAME: &str = "Controls";
#[cfg(target_os = "windows")]
const MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID: &str = "main-window:control:set-ready";
#[cfg(target_os = "windows")]
const MAIN_WINDOW_LOCAL_READY_BUTTON_NAME: &str = "Set Ready";
#[cfg(target_os = "windows")]
const MAIN_WINDOW_ROOM_BROWSER_NAME: &str = "Room Browser";
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_X: i32 = 32;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_Y: i32 = 32;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_WIDTH: i32 = 1700;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_HEIGHT: i32 = 1100;

const BENCH_USERNAME: &str = "smoke-user";
const BENCH_ROOM: &str = "smoke-room";
const DEFAULT_PUBLIC_SERVERS_SPEC: &str =
    "[['Bench Primary', '127.0.0.1:8999'], ['Bench Backup', '127.0.0.1:9000']]";
const DEFAULT_UPDATE_CHECK_RESPONSE: &str =
    r#"{"version-status":"uptodate","version-message":"Sorotte is up to date."}"#;
const DLL_INIT_FAILED_STATUS: u32 = 0xC000_0142;
const LAUNCH_ATTEMPTS: usize = 2;

#[path = "sorotte-gui-native-smoke/platform_driver.rs"]
mod platform_driver;
use platform_driver::{NativeGuiDriver, PlatformNativeGuiDriver};

fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StartupScenario {
    FirstRun,
    Configured,
    TcpConnect,
    ProfileCopy,
}

impl StartupScenario {
    fn label(self) -> &'static str {
        match self {
            Self::FirstRun => "first-run",
            Self::Configured => "configured",
            Self::TcpConnect => "tcp-connect",
            Self::ProfileCopy => "profile-copy",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "first-run" | "firstrun" => Ok(Self::FirstRun),
            "configured" => Ok(Self::Configured),
            "tcp-connect" | "tcp" | "connect" => Ok(Self::TcpConnect),
            "profile-copy" | "profile" | "real-profile" => Ok(Self::ProfileCopy),
            other => Err(format!(
                "unknown scenario {other:?}; expected first-run, configured, tcp-connect, or profile-copy"
            )),
        }
    }
}

#[derive(Debug)]
struct StartupBenchOptions {
    binary_path: PathBuf,
    samples: usize,
    warmup: usize,
    timeout: Duration,
    scenarios: Vec<StartupScenario>,
    keep_profile_copy: bool,
    json: bool,
    compare_to: Option<PathBuf>,
    output_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartupMetrics {
    window_visible_ms: f64,
    first_usable_gui_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    connected_session_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tcp_hello_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartupRun {
    binary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_revision: Option<String>,
    scenario: StartupScenario,
    sample_index: usize,
    warmup: bool,
    run_id: String,
    metrics: StartupMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricStats {
    count: usize,
    first_ms: f64,
    median_ms: f64,
    p90_ms: f64,
    min_ms: f64,
    max_ms: f64,
    stddev_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenarioSummary {
    scenario: StartupScenario,
    metrics: BTreeMap<String, MetricStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ComparisonRow {
    scenario: StartupScenario,
    metric: String,
    baseline_median_ms: f64,
    current_median_ms: f64,
    delta_ms: f64,
    delta_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartupBenchmarkResults {
    schema_version: u32,
    generated_at_unix_ms: u128,
    output_dir: String,
    binary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_revision: Option<String>,
    samples: usize,
    warmup: usize,
    timeout_ms: u64,
    runs: Vec<StartupRun>,
    summary: Vec<ScenarioSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comparison: Option<Vec<ComparisonRow>>,
}

#[derive(Clone, Copy)]
struct GuiLaunchConfig<'a> {
    config_path: &'a Path,
    public_servers_spec: Option<&'a str>,
    update_check_response: Option<&'a str>,
    test_player: bool,
    tcp_session: Option<TcpSessionBootstrap>,
}

struct StartupSampleRequest<'a> {
    binary_path: &'a Path,
    git_revision: Option<&'a str>,
    output_dir: &'a Path,
    scenario: StartupScenario,
    sample_index: usize,
    warmup: bool,
    timeout: Duration,
    keep_profile_copy: bool,
}

#[derive(Clone, Copy)]
struct TcpSessionBootstrap {
    port: u16,
}

struct MockHello {
    received_at: Instant,
    line: String,
}

struct MockSessionServer {
    address: String,
    port: u16,
    hello_rx: mpsc::Receiver<MockHello>,
    release_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl MockSessionServer {
    fn recv_hello(&self, timeout: Duration) -> Result<MockHello, String> {
        self.hello_rx
            .recv_timeout(timeout)
            .map_err(|error| format!("timed out waiting for mock TCP startup Hello: {error}"))
    }

    fn release(mut self) -> Result<(), String> {
        let _ = self.release_tx.send(());
        let Some(join_handle) = self.join_handle.take() else {
            return Ok(());
        };
        match join_handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(format!("mock TCP server failed: {error}")),
            Err(_) => Err("mock TCP server thread panicked".to_owned()),
        }
    }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    };

    match run_benchmark(&options) {
        Ok(results) => {
            if options.json {
                match serde_json::to_string_pretty(&results) {
                    Ok(rendered) => println!("{rendered}"),
                    Err(error) => {
                        eprintln!("failed to render startup benchmark JSON: {error}");
                        std::process::exit(1);
                    }
                }
            } else {
                print!("{}", render_text_report(&results));
            }
        }
        Err(error) => {
            eprintln!("sorotte-gui-startup-bench failed: {error}");
            std::process::exit(1);
        }
    }
}

fn usage() -> &'static str {
    "usage: sorotte-gui-startup-bench --binary PATH [--samples N] [--warmup N] [--timeout-ms N] [--scenario NAME] [--keep-profile-copy] [--json] [--compare-to PATH] [--output-dir PATH]"
}

fn parse_options(args: &[String]) -> Result<StartupBenchOptions, String> {
    let mut binary_path = None;
    let mut samples = 20usize;
    let mut warmup = 3usize;
    let mut timeout = Duration::from_millis(10_000);
    let mut scenarios = Vec::new();
    let mut keep_profile_copy = false;
    let mut json = false;
    let mut compare_to = None;
    let mut output_dir = None;

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--binary" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--binary requires a path".to_owned())?;
                binary_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--samples" => {
                samples = parse_positive_usize(args.get(index + 1), "--samples", true)?;
                index += 2;
            }
            "--warmup" => {
                warmup = parse_positive_usize(args.get(index + 1), "--warmup", true)?;
                index += 2;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--timeout-ms requires an integer".to_owned())?;
                let timeout_ms = value
                    .parse::<u64>()
                    .map_err(|_| format!("--timeout-ms requires an integer, got {value:?}"))?;
                if timeout_ms == 0 {
                    return Err("--timeout-ms must be greater than zero".to_owned());
                }
                timeout = Duration::from_millis(timeout_ms);
                index += 2;
            }
            "--scenario" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--scenario requires a name".to_owned())?;
                for scenario in value.split(',') {
                    let scenario = scenario.trim();
                    if !scenario.is_empty() {
                        scenarios.push(StartupScenario::parse(scenario)?);
                    }
                }
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            "--keep-profile-copy" => {
                keep_profile_copy = true;
                index += 1;
            }
            "--compare-to" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--compare-to requires a path".to_owned())?;
                compare_to = Some(PathBuf::from(value));
                index += 2;
            }
            "--output-dir" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--output-dir requires a path".to_owned())?;
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    let Some(binary_path) = binary_path else {
        return Err("--binary is required".to_owned());
    };
    if samples == 0 {
        return Err("--samples must be greater than zero".to_owned());
    }
    if scenarios.is_empty() {
        scenarios = vec![
            StartupScenario::FirstRun,
            StartupScenario::Configured,
            StartupScenario::TcpConnect,
        ];
    }
    scenarios.sort();
    scenarios.dedup();

    Ok(StartupBenchOptions {
        binary_path,
        samples,
        warmup,
        timeout,
        scenarios,
        keep_profile_copy,
        json,
        compare_to,
        output_dir,
    })
}

fn parse_positive_usize(
    value: Option<&String>,
    label: &str,
    allow_zero: bool,
) -> Result<usize, String> {
    let value = value.ok_or_else(|| format!("{label} requires an integer"))?;
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} requires an integer, got {value:?}"))?;
    if !allow_zero && parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn run_benchmark(options: &StartupBenchOptions) -> Result<StartupBenchmarkResults, String> {
    let binary_path = fs::canonicalize(&options.binary_path).map_err(|error| {
        format!(
            "failed to resolve sorotte-gui binary {}: {error}",
            options.binary_path.display()
        )
    })?;
    if !binary_path.is_file() {
        return Err(format!(
            "sorotte-gui binary is not a file: {}",
            binary_path.display()
        ));
    }

    let output_dir = options
        .output_dir
        .clone()
        .unwrap_or_else(default_output_dir);
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create startup benchmark output directory {}: {error}",
            output_dir.display()
        )
    })?;
    let output_dir = fs::canonicalize(&output_dir).map_err(|error| {
        format!(
            "failed to resolve startup benchmark output directory {}: {error}",
            output_dir.display()
        )
    })?;

    let git_revision = git_revision();
    let mut runs = Vec::new();
    for scenario in &options.scenarios {
        let total = options.warmup + options.samples;
        for sample_index in 0..total {
            let warmup = sample_index < options.warmup;
            let run = run_sample(StartupSampleRequest {
                binary_path: &binary_path,
                git_revision: git_revision.as_deref(),
                output_dir: &output_dir,
                scenario: *scenario,
                sample_index,
                warmup,
                timeout: options.timeout,
                keep_profile_copy: options.keep_profile_copy,
            })?;
            runs.push(run);
        }
    }

    let summary = summarize_runs(&runs);
    let comparison = if let Some(compare_to) = options.compare_to.as_ref() {
        let baseline = load_results(compare_to)?;
        Some(compare_summaries(&summary, &baseline.summary))
    } else {
        None
    };

    let results = StartupBenchmarkResults {
        schema_version: 2,
        generated_at_unix_ms: unix_ms(),
        output_dir: output_dir.display().to_string(),
        binary: binary_path.display().to_string(),
        git_revision,
        samples: options.samples,
        warmup: options.warmup,
        timeout_ms: options.timeout.as_millis() as u64,
        runs,
        summary,
        comparison,
    };

    let results_path = output_dir.join("results.json");
    let rendered = serde_json::to_string_pretty(&results)
        .map_err(|error| format!("failed to render startup benchmark results JSON: {error}"))?;
    fs::write(&results_path, rendered).map_err(|error| {
        format!(
            "failed to write startup benchmark results {}: {error}",
            results_path.display()
        )
    })?;

    Ok(results)
}

fn default_output_dir() -> PathBuf {
    PathBuf::from("target")
        .join("startup-bench")
        .join(unix_ms().to_string())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn git_revision() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_sample(request: StartupSampleRequest<'_>) -> Result<StartupRun, String> {
    let StartupSampleRequest {
        binary_path,
        git_revision,
        output_dir,
        scenario,
        sample_index,
        warmup,
        timeout,
        keep_profile_copy,
    } = request;

    let run_id = format!(
        "{}-{}-{}",
        scenario.label(),
        if warmup { "warmup" } else { "sample" },
        sample_index
    );
    let sample_root = output_dir.join(&run_id);
    fs::create_dir_all(&sample_root).map_err(|error| {
        format!(
            "failed to create benchmark sample directory {}: {error}",
            sample_root.display()
        )
    })?;
    let config_path = sample_root.join("sorotte-bench.ini");
    let mock_server = if scenario == StartupScenario::TcpConnect {
        Some(start_mock_session_server()?)
    } else {
        None
    };
    if let Err(error) = prepare_scenario_files(
        scenario,
        &sample_root,
        &config_path,
        mock_server.as_ref().map(|server| server.port),
    ) {
        if let Some(server) = mock_server {
            let _ = server.release();
        }
        if scenario == StartupScenario::ProfileCopy && !keep_profile_copy {
            let _ = cleanup_profile_copy_sample(&sample_root, &config_path);
        }
        return Err(error);
    }
    let tcp_session = mock_server
        .as_ref()
        .map(|server| TcpSessionBootstrap { port: server.port });
    let public_servers_spec = mock_server
        .as_ref()
        .map(|server| format!("[['Bench Local', '{}']]", server.address));
    let use_real_profile = scenario == StartupScenario::ProfileCopy;
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        public_servers_spec: if use_real_profile {
            None
        } else {
            public_servers_spec
                .as_deref()
                .or(Some(DEFAULT_PUBLIC_SERVERS_SPEC))
        },
        update_check_response: (!use_real_profile).then_some(DEFAULT_UPDATE_CHECK_RESPONSE),
        test_player: matches!(
            scenario,
            StartupScenario::Configured | StartupScenario::TcpConnect
        ),
        tcp_session,
    };

    let driver = PlatformNativeGuiDriver;
    let started_at = Instant::now();
    let mut child = match launch_sorotte_gui_with_retry(&driver, binary_path, launch, timeout) {
        Ok(child) => child,
        Err(error) => {
            if let Some(server) = mock_server {
                let _ = server.release();
            }
            if scenario == StartupScenario::ProfileCopy && !keep_profile_copy {
                let _ = cleanup_profile_copy_sample(&sample_root, &config_path);
            }
            return Err(error);
        }
    };
    let pid = child.id();

    let result = (|| {
        let window_visible_ms = wait_for_main_window(&driver, &mut child, started_at, timeout)?;
        if let Some(window) = driver.find_main_window(pid)? {
            let _ = driver.prepare_window_for_smoke(window);
            let first_usable_gui_ms =
                wait_for_first_usable_gui(&driver, window, scenario, started_at, timeout)?;
            let (connected_session_ms, tcp_hello_ms) = if let Some(server) = mock_server.as_ref() {
                let connected_session_ms =
                    wait_for_connected_session(&driver, window, started_at, timeout)?;
                let hello = server.recv_hello(Duration::from_millis(1_500))?;
                if !hello.line.contains("\"Hello\"") {
                    return Err(format!(
                        "mock TCP server received unexpected startup payload: {:?}",
                        hello.line
                    ));
                }
                (
                    Some(connected_session_ms),
                    Some(duration_ms(started_at, hello.received_at)),
                )
            } else {
                (None, None)
            };

            driver.close_window(window)?;
            wait_for_process_exit(&mut child, timeout)?;
            Ok(StartupRun {
                binary: binary_path.display().to_string(),
                git_revision: git_revision.map(str::to_owned),
                scenario,
                sample_index,
                warmup,
                run_id: run_id.clone(),
                metrics: StartupMetrics {
                    window_visible_ms,
                    first_usable_gui_ms,
                    connected_session_ms,
                    tcp_hello_ms,
                },
            })
        } else {
            Err("main window disappeared after it was discovered".to_owned())
        }
    })();

    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    if let Some(server) = mock_server {
        server.release()?;
    }

    if scenario == StartupScenario::ProfileCopy && !keep_profile_copy {
        let cleanup_result = cleanup_profile_copy_sample(&sample_root, &config_path);
        if result.is_ok() {
            cleanup_result?;
        }
    }

    result
}

fn prepare_scenario_files(
    scenario: StartupScenario,
    sample_root: &Path,
    config_path: &Path,
    tcp_port: Option<u16>,
) -> Result<(), String> {
    let _ = fs::remove_file(config_path);
    match scenario {
        StartupScenario::FirstRun => Ok(()),
        StartupScenario::Configured => {
            seed_config(config_path, None)?;
            write_gui_active_view(sample_root, "room")
        }
        StartupScenario::TcpConnect => {
            seed_config(config_path, tcp_port)?;
            write_gui_active_view(sample_root, "room")
        }
        StartupScenario::ProfileCopy => copy_real_profile_for_sample(sample_root, config_path),
    }
}

fn copy_real_profile_for_sample(sample_root: &Path, config_path: &Path) -> Result<(), String> {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "profile-copy requires APPDATA to locate the real Sorotte profile".to_owned()
        })?;
    let source_root = appdata.join("Sorotte");
    let source_config = source_root.join("sorotte.ini");
    if !source_config.is_file() {
        return Err(format!(
            "profile-copy could not find real Sorotte config {}",
            source_config.display()
        ));
    }
    copy_file_to_path(&source_config, config_path, "real Sorotte config")?;

    let source_main_window = source_root.join("MainWindow.ini");
    if source_main_window.is_file() {
        copy_file_to_path(
            &source_main_window,
            &sample_root.join("MainWindow.ini"),
            "real Sorotte main-window state",
        )?;
    }

    let source_stream_helper = source_root.join("tools").join("stream-helper");
    if source_stream_helper.is_dir() {
        copy_dir_recursive(
            &source_stream_helper,
            &sample_root.join("tools").join("stream-helper"),
        )?;
    }

    Ok(())
}

fn copy_file_to_path(source: &Path, destination: &Path, context: &str) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create destination directory for {context} {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::copy(source, destination).map_err(|error| {
        format!(
            "failed to copy {context} from {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create copied profile directory {}: {error}",
            destination.display()
        )
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        format!(
            "failed to read copied profile directory {}: {error}",
            source.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read copied profile entry in {}: {error}",
                source.display()
            )
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect copied profile entry {}: {error}",
                source_path.display()
            )
        })?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            copy_file_to_path(&source_path, &destination_path, "copied profile file")?;
        }
    }
    Ok(())
}

fn cleanup_profile_copy_sample(sample_root: &Path, config_path: &Path) -> Result<(), String> {
    match remove_file_with_retries(config_path, 25, Duration::from_millis(100)) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to remove copied profile config {}: {error}",
                config_path.display()
            ));
        }
    }

    let tools_root = sample_root.join("tools");
    match remove_dir_all_with_retries(&tools_root, 50, Duration::from_millis(100)) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to remove copied profile tools {}: {error}",
                tools_root.display()
            ));
        }
    }
    let main_window_path = sample_root.join("MainWindow.ini");
    match remove_file_with_retries(&main_window_path, 25, Duration::from_millis(100)) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to remove copied profile GUI state {}: {error}",
                main_window_path.display()
            ));
        }
    }
    Ok(())
}

fn remove_file_with_retries(path: &Path, attempts: usize, delay: Duration) -> std::io::Result<()> {
    retry_io(attempts, delay, || fs::remove_file(path))
}

fn remove_dir_all_with_retries(
    path: &Path,
    attempts: usize,
    delay: Duration,
) -> std::io::Result<()> {
    retry_io(attempts, delay, || fs::remove_dir_all(path))
}

fn retry_io<F>(attempts: usize, delay: Duration, mut operation: F) -> std::io::Result<()>
where
    F: FnMut() -> std::io::Result<()>,
{
    let attempts = attempts.max(1);
    for attempt in 1..=attempts {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(error),
            Err(error) if attempt == attempts => return Err(error),
            Err(_) => std::thread::sleep(delay),
        }
    }
    Ok(())
}

fn seed_config(config_path: &Path, port: Option<u16>) -> Result<(), String> {
    let settings = StoredClientSettingsMvp {
        host: port.map(|_| "127.0.0.1".to_owned()),
        port,
        username: Some(BENCH_USERNAME.to_owned()),
        room: Some(BENCH_ROOM.to_owned()),
        player_path: Some("mpv".to_owned()),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        check_for_updates_automatically: Some(false),
        public_servers: Some(vec![
            ("Bench Primary".to_owned(), "127.0.0.1:8999".to_owned()),
            ("Bench Backup".to_owned(), "127.0.0.1:9000".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(|error| {
        format!(
            "failed to seed benchmark config {}: {error}",
            config_path.display()
        )
    })
}

fn write_gui_active_view(root: &Path, active_view: &str) -> Result<(), String> {
    let path = root.join("MainWindow.ini");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create benchmark GUI state directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(
        &path,
        format!("[MainWindow]\nactiveView = {active_view}\n\n"),
    )
    .map_err(|error| {
        format!(
            "failed to write benchmark GUI state {}: {error}",
            path.display()
        )
    })
}

fn launch_sorotte_gui(binary_path: &Path, launch: GuiLaunchConfig<'_>) -> Result<Child, String> {
    let mut command = Command::new(binary_path);
    if let Some(parent) = binary_path.parent() {
        command.current_dir(parent);
    }
    for name in [
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP",
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK",
        "SOROTTE_GUI_ENABLE_TEST_PLAYER",
        "SOROTTE_CLIENT_HOST",
        "SOROTTE_CLIENT_PORT",
        "SOROTTE_CLIENT_USERNAME",
        "SOROTTE_CLIENT_NAME",
        "SOROTTE_CLIENT_ROOM",
        "SOROTTE_CLIENT_MPV_IPC_PATH",
        "SOROTTE_MPV_IPC_PATH",
        "SOROTTE_GUI_TEST_DROP_FILE_PATHS",
        "SOROTTE_GUI_TEST_DROP_TARGET",
        "SOROTTE_GUI_UPDATE_CHECK_RESPONSE",
        "SOROTTE_GUI_PUBLIC_SERVER_LIST_URL",
        "SOROTTE_GUI_PUBLIC_SERVER_LIST_RESPONSE",
        "SOROTTE_GUI_UPDATE_CHECK_URL",
        "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS",
        "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH",
    ] {
        command.env_remove(name);
    }
    command.env("SOROTTE_CLIENT_CONFIG_PATH", launch.config_path);
    if let Some(update_check_response) = launch.update_check_response {
        command.env("SOROTTE_GUI_UPDATE_CHECK_RESPONSE", update_check_response);
    }
    if let Some(public_servers_spec) = launch.public_servers_spec {
        command.env("SOROTTE_GUI_REFRESH_PUBLIC_SERVERS", public_servers_spec);
    }
    if launch.test_player {
        command.env("SOROTTE_GUI_ENABLE_TEST_PLAYER", "true");
    }
    if let Some(tcp_session) = launch.tcp_session {
        command.env("SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP", "true");
        command.env("SOROTTE_CLIENT_HOST", "127.0.0.1");
        command.env("SOROTTE_CLIENT_PORT", tcp_session.port.to_string());
        command.env("SOROTTE_CLIENT_USERNAME", BENCH_USERNAME);
        command.env("SOROTTE_CLIENT_ROOM", BENCH_ROOM);
    }
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command
        .spawn()
        .map_err(|error| format!("failed to launch sorotte-gui at {binary_path:?}: {error}"))
}

fn launch_sorotte_gui_with_retry<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    timeout: Duration,
) -> Result<Child, String> {
    let mut last_error = String::new();
    for attempt in 1..=LAUNCH_ATTEMPTS {
        let mut child = launch_sorotte_gui(binary_path, GuiLaunchConfig { ..launch })?;
        match wait_for_process_or_window(driver, &mut child, timeout) {
            Ok(()) => return Ok(child),
            Err(error) => {
                let retryable = child
                    .try_wait()
                    .ok()
                    .flatten()
                    .and_then(|status| status.code())
                    .is_some_and(|status| status as u32 == DLL_INIT_FAILED_STATUS);
                last_error = error;
                let _ = child.kill();
                let _ = child.wait();
                if retryable && attempt < LAUNCH_ATTEMPTS {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                break;
            }
        }
    }
    Err(last_error)
}

fn wait_for_process_or_window<D: NativeGuiDriver>(
    driver: &D,
    child: &mut Child,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll sorotte-gui process state: {error}"))?
        {
            return Err(format!(
                "sorotte-gui exited before exposing a main window (status: {status})"
            ));
        }
        if driver.find_main_window(child.id())?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the sorotte-gui main window".to_owned());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_main_window<D: NativeGuiDriver>(
    driver: &D,
    child: &mut Child,
    started_at: Instant,
    timeout: Duration,
) -> Result<f64, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll sorotte-gui process state: {error}"))?
        {
            return Err(format!(
                "sorotte-gui exited before exposing a main window (status: {status})"
            ));
        }
        if driver.find_main_window(child.id())?.is_some() {
            return Ok(elapsed_ms(started_at));
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the sorotte-gui main window".to_owned());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_first_usable_gui<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    scenario: StartupScenario,
    started_at: Instant,
    timeout: Duration,
) -> Result<f64, String> {
    wait_for_accessibility_match(
        driver,
        window,
        started_at,
        timeout,
        match scenario {
            StartupScenario::FirstRun => "view: setup",
            StartupScenario::Configured => "view: setup or view: room",
            StartupScenario::TcpConnect => "view: setup or view: room",
            StartupScenario::ProfileCopy => "view: setup or view: room",
        },
        |names| match scenario {
            StartupScenario::FirstRun => contains_name(names, "view: setup"),
            StartupScenario::Configured => {
                contains_name(names, "view: setup") || contains_name(names, "view: room")
            }
            StartupScenario::TcpConnect => {
                contains_name(names, "view: setup") || contains_name(names, "view: room")
            }
            StartupScenario::ProfileCopy => {
                contains_name(names, "view: setup") || contains_name(names, "view: room")
            }
        },
    )
}

fn wait_for_connected_session<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    started_at: Instant,
    timeout: Duration,
) -> Result<f64, String> {
    wait_for_accessibility_match(
        driver,
        window,
        started_at,
        timeout,
        "view: room with local user",
        |names| contains_name(names, "view: room") && contains_name(names, BENCH_USERNAME),
    )
}

fn wait_for_accessibility_match<D, F>(
    driver: &D,
    window: D::WindowHandle,
    started_at: Instant,
    timeout: Duration,
    label: &str,
    predicate: F,
) -> Result<f64, String>
where
    D: NativeGuiDriver,
    F: Fn(&[String]) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if predicate(&names) {
                    return Ok(elapsed_ms(started_at));
                }
                last_snapshot = Some(render_accessible_snapshot(&names));
            }
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for {label}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for {label}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn contains_name(names: &[String], expected: &str) -> bool {
    names.iter().any(|name| name == expected)
}

fn render_accessible_snapshot(names: &[String]) -> String {
    let patterns = [
        "view:",
        "modal:",
        "Status",
        "Connected",
        "Disconnected",
        BENCH_USERNAME,
        BENCH_ROOM,
        "Connect",
        "Save",
        "pending:",
    ];
    let snapshot = names
        .iter()
        .filter(|name| patterns.iter().any(|pattern| name.contains(pattern)))
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>();
    if snapshot.is_empty() {
        "none".to_owned()
    } else {
        snapshot.join(", ")
    }
}

fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll sorotte-gui exit state: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for sorotte-gui to exit after close request".to_owned());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn elapsed_ms(started_at: Instant) -> f64 {
    started_at.elapsed().as_secs_f64() * 1000.0
}

fn duration_ms(started_at: Instant, ended_at: Instant) -> f64 {
    ended_at.duration_since(started_at).as_secs_f64() * 1000.0
}

fn start_mock_session_server() -> Result<MockSessionServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind mock TCP listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set mock TCP listener nonblocking mode: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read mock TCP listener address: {error}"))?;
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, _) = loop {
            if release_rx.try_recv().is_ok() {
                return Ok(());
            }
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= accept_deadline {
                        return Err(
                            "mock TCP server timed out waiting for client connection".to_owned()
                        );
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(format!("mock TCP server failed to accept: {error}")),
            }
        };
        configure_mock_stream(&stream)?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("mock TCP server failed to clone stream: {error}"))?;
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_startup_hello_line(&mut stream, &mut reader)?;
        hello_tx
            .send(MockHello {
                received_at: Instant::now(),
                line: hello_line,
            })
            .map_err(|error| format!("mock TCP server failed to report Hello: {error}"))?;
        stream
            .write_all(
                format!(
                    r#"{{"Hello":{{"username":"{BENCH_USERNAME}","room":{{"name":"{BENCH_ROOM}"}},"version":"1.7.5","features":{{"chat":true,"readiness":true,"sharedPlaylists":true}}}}}}"#
                )
                .as_bytes(),
            )
            .map_err(|error| format!("mock TCP server failed to write Hello: {error}"))?;
        stream
            .write_all(b"\n")
            .map_err(|error| format!("mock TCP server failed to terminate Hello: {error}"))?;

        let _ = release_rx.recv_timeout(Duration::from_secs(10));
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port: address.port(),
        hello_rx,
        release_tx,
        join_handle: Some(join_handle),
    })
}

fn configure_mock_stream(stream: &TcpStream) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("mock TCP server failed to restore blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("mock TCP server failed to set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("mock TCP server failed to set write timeout: {error}"))?;
    Ok(())
}

fn read_startup_hello_line(
    stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
) -> Result<String, String> {
    let mut hello_line = String::new();
    reader
        .read_line(&mut hello_line)
        .map_err(|error| format!("mock TCP server failed to read startup line: {error}"))?;
    if hello_line.contains("\"startTLS\":\"send\"") {
        stream
            .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
            .map_err(|error| {
                format!("mock TCP server failed to decline TLS negotiation: {error}")
            })?;
        stream.write_all(b"\n").map_err(|error| {
            format!("mock TCP server failed to terminate TLS response: {error}")
        })?;
        hello_line.clear();
        reader
            .read_line(&mut hello_line)
            .map_err(|error| format!("mock TCP server failed to read post-TLS Hello: {error}"))?;
    }
    Ok(hello_line)
}

fn summarize_runs(runs: &[StartupRun]) -> Vec<ScenarioSummary> {
    let mut by_scenario = BTreeMap::<StartupScenario, Vec<&StartupRun>>::new();
    for run in runs.iter().filter(|run| !run.warmup) {
        by_scenario.entry(run.scenario).or_default().push(run);
    }
    by_scenario
        .into_iter()
        .map(|(scenario, runs)| {
            let mut metrics = BTreeMap::new();
            insert_metric_stats(
                &mut metrics,
                "window_visible_ms",
                runs.iter().map(|run| Some(run.metrics.window_visible_ms)),
            );
            insert_metric_stats(
                &mut metrics,
                "first_usable_gui_ms",
                runs.iter().map(|run| Some(run.metrics.first_usable_gui_ms)),
            );
            insert_metric_stats(
                &mut metrics,
                "connected_session_ms",
                runs.iter().map(|run| run.metrics.connected_session_ms),
            );
            insert_metric_stats(
                &mut metrics,
                "tcp_hello_ms",
                runs.iter().map(|run| run.metrics.tcp_hello_ms),
            );
            ScenarioSummary { scenario, metrics }
        })
        .collect()
}

fn insert_metric_stats<I>(metrics: &mut BTreeMap<String, MetricStats>, name: &str, values: I)
where
    I: IntoIterator<Item = Option<f64>>,
{
    let values = values.into_iter().flatten().collect::<Vec<_>>();
    if let Some(stats) = metric_stats(&values) {
        metrics.insert(name.to_owned(), stats);
    }
}

fn metric_stats(values: &[f64]) -> Option<MetricStats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let first_ms = values[0];
    let median_ms = if count.is_multiple_of(2) {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    } else {
        sorted[count / 2]
    };
    let p90_ms = percentile_nearest_rank(&sorted, 90.0);
    let min_ms = sorted[0];
    let max_ms = sorted[count - 1];
    let mean = values.iter().sum::<f64>() / count as f64;
    let variance = values
        .iter()
        .map(|value| {
            let diff = *value - mean;
            diff * diff
        })
        .sum::<f64>()
        / count as f64;
    Some(MetricStats {
        count,
        first_ms,
        median_ms,
        p90_ms,
        min_ms,
        max_ms,
        stddev_ms: variance.sqrt(),
    })
}

fn percentile_nearest_rank(sorted_values: &[f64], percentile: f64) -> f64 {
    debug_assert!(!sorted_values.is_empty());
    let rank = ((percentile / 100.0) * sorted_values.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted_values.len() - 1);
    sorted_values[index]
}

fn load_results(path: &Path) -> Result<StartupBenchmarkResults, String> {
    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read comparison baseline {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        format!(
            "failed to parse comparison baseline {}: {error}",
            path.display()
        )
    })
}

fn compare_summaries(
    current: &[ScenarioSummary],
    baseline: &[ScenarioSummary],
) -> Vec<ComparisonRow> {
    let baseline_by_scenario = baseline
        .iter()
        .map(|summary| (summary.scenario, summary))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    for current_summary in current {
        let Some(baseline_summary) = baseline_by_scenario.get(&current_summary.scenario) else {
            continue;
        };
        for (metric, current_stats) in &current_summary.metrics {
            let Some(baseline_stats) = baseline_summary.metrics.get(metric) else {
                continue;
            };
            let delta_ms = current_stats.median_ms - baseline_stats.median_ms;
            let delta_percent = if baseline_stats.median_ms.abs() > f64::EPSILON {
                (delta_ms / baseline_stats.median_ms) * 100.0
            } else {
                0.0
            };
            rows.push(ComparisonRow {
                scenario: current_summary.scenario,
                metric: metric.clone(),
                baseline_median_ms: baseline_stats.median_ms,
                current_median_ms: current_stats.median_ms,
                delta_ms,
                delta_percent,
            });
        }
    }
    rows
}

fn render_text_report(results: &StartupBenchmarkResults) -> String {
    let mut out = String::new();
    out.push_str("Sorotte GUI startup benchmark\n");
    out.push_str(&format!("results={}\n", results.output_dir));
    out.push_str(&format!("binary={}\n", results.binary));
    if let Some(git_revision) = results.git_revision.as_deref() {
        out.push_str(&format!("git_revision={git_revision}\n"));
    }
    out.push('\n');
    for summary in &results.summary {
        out.push_str(&format!("{}\n", summary.scenario.label()));
        out.push_str("metric\tcount\tfirst\tmedian\tp90\tmin\tmax\tstddev\n");
        for (name, stats) in &summary.metrics {
            out.push_str(&format!(
                "{name}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\n",
                stats.count,
                stats.first_ms,
                stats.median_ms,
                stats.p90_ms,
                stats.min_ms,
                stats.max_ms,
                stats.stddev_ms
            ));
        }
        out.push('\n');
    }
    if let Some(comparison) = results.comparison.as_ref()
        && !comparison.is_empty()
    {
        out.push_str("comparison\n");
        out.push_str("scenario\tmetric\tbaseline_median\tcurrent_median\tdelta\tdelta_percent\n");
        for row in comparison {
            out.push_str(&format!(
                "{}\t{}\t{:.1}\t{:.1}\t{:+.1}\t{:+.1}%\n",
                row.scenario.label(),
                row.metric,
                row.baseline_median_ms,
                row.current_median_ms,
                row.delta_ms,
                row.delta_percent
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_options_accepts_gated_profile_copy_scenario() {
        let options = parse_options(&[
            "--binary".to_owned(),
            "sorotte-gui.exe".to_owned(),
            "--scenario".to_owned(),
            "profile-copy".to_owned(),
            "--keep-profile-copy".to_owned(),
        ])
        .expect("profile-copy options should parse");

        assert_eq!(options.scenarios, vec![StartupScenario::ProfileCopy]);
        assert!(options.keep_profile_copy);
    }

    #[test]
    fn parse_options_default_scenarios_do_not_include_profile_copy() {
        let options = parse_options(&["--binary".to_owned(), "sorotte-gui.exe".to_owned()])
            .expect("default options should parse");

        assert_eq!(
            options.scenarios,
            vec![
                StartupScenario::FirstRun,
                StartupScenario::Configured,
                StartupScenario::TcpConnect,
            ]
        );
    }

    #[test]
    fn metric_stats_reports_median_p90_and_stddev() {
        let stats = metric_stats(&[10.0, 20.0, 30.0, 40.0]).expect("stats should exist");

        assert_eq!(stats.count, 4);
        assert_eq!(stats.first_ms, 10.0);
        assert_eq!(stats.median_ms, 25.0);
        assert_eq!(stats.p90_ms, 40.0);
        assert_eq!(stats.min_ms, 10.0);
        assert_eq!(stats.max_ms, 40.0);
        assert!((stats.stddev_ms - 11.180339887).abs() < 0.0001);
    }

    #[test]
    fn comparison_uses_median_delta() {
        let current = vec![ScenarioSummary {
            scenario: StartupScenario::FirstRun,
            metrics: BTreeMap::from([(
                "window_visible_ms".to_owned(),
                MetricStats {
                    count: 1,
                    first_ms: 90.0,
                    median_ms: 90.0,
                    p90_ms: 90.0,
                    min_ms: 90.0,
                    max_ms: 90.0,
                    stddev_ms: 0.0,
                },
            )]),
        }];
        let baseline = vec![ScenarioSummary {
            scenario: StartupScenario::FirstRun,
            metrics: BTreeMap::from([(
                "window_visible_ms".to_owned(),
                MetricStats {
                    count: 1,
                    first_ms: 100.0,
                    median_ms: 100.0,
                    p90_ms: 100.0,
                    min_ms: 100.0,
                    max_ms: 100.0,
                    stddev_ms: 0.0,
                },
            )]),
        }];

        let rows = compare_summaries(&current, &baseline);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].delta_ms, -10.0);
        assert_eq!(rows[0].delta_percent, -10.0);
    }
}
