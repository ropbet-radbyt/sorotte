use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;
use crate::platform_driver::NativeAccessibilityNode;

const VISUAL_SCHEMA_VERSION: u32 = 1;
const VISUAL_SETTLE_DELAY: Duration = Duration::from_millis(800);
const VISUAL_VIEWPORT_WIDTH: i32 = 1700;
const VISUAL_VIEWPORT_HEIGHT: i32 = 1100;
const CONNECTION_HOST_AUTOMATION_ID: &str = "settings.connection.host";
const CONNECTION_PORT_AUTOMATION_ID: &str = "settings.connection.port";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisualScenario {
    FirstRunPlayerMissing,
    ConnectionClean,
    ConnectionDirty,
    ValidationErrors,
}

impl VisualScenario {
    const ALL: [Self; 4] = [
        Self::FirstRunPlayerMissing,
        Self::ConnectionClean,
        Self::ConnectionDirty,
        Self::ValidationErrors,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::FirstRunPlayerMissing => "settings.first-run.player-missing",
            Self::ConnectionClean => "settings.connection.clean",
            Self::ConnectionDirty => "settings.connection.dirty",
            Self::ValidationErrors => "settings.validation-errors",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.id() == value.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                format!(
                    "unknown visual scenario {value:?}; expected {}",
                    Self::ALL
                        .iter()
                        .map(|scenario| scenario.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    fn first_run(self) -> bool {
        self == Self::FirstRunPlayerMissing
    }

    fn selected_page(self) -> &'static str {
        if self.first_run() {
            "first-run-player-remediation"
        } else {
            "connection"
        }
    }

    fn dirty_settings(self) -> &'static [&'static str] {
        match self {
            Self::ConnectionDirty => &["connection.host"],
            Self::ValidationErrors => &["connection.port"],
            Self::FirstRunPlayerMissing | Self::ConnectionClean => &[],
        }
    }
}

struct VisualSuiteOptions {
    binary_path: PathBuf,
    output_dir: PathBuf,
    timeout: Duration,
    scenarios: Vec<VisualScenario>,
}

#[derive(Serialize)]
struct ViewportManifest {
    width: i32,
    height: i32,
    scale: &'static str,
}

#[derive(Serialize)]
struct ArtifactManifest {
    window: ArtifactFileManifest,
    semantics: ArtifactFileManifest,
}

#[derive(Serialize)]
struct ArtifactFileManifest {
    path: &'static str,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct DeterminismManifest {
    viewport: &'static str,
    configuration: &'static str,
    public_servers: &'static str,
    theme: &'static str,
    scale: &'static str,
    fonts: &'static str,
}

#[derive(Serialize)]
struct ScenarioManifest<'a> {
    schema_version: u32,
    scenario: &'a str,
    git_sha: &'a str,
    git_dirty: bool,
    theme: &'static str,
    viewport: ViewportManifest,
    selected_page: &'static str,
    dirty_settings: &'static [&'static str],
    focused_element: Option<&'a str>,
    interaction_target: Option<&'a str>,
    artifacts: ArtifactManifest,
    determinism: DeterminismManifest,
}

#[derive(Serialize)]
struct SemanticBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Serialize)]
struct SemanticNode<'a> {
    index: usize,
    name: &'a str,
    automation_id: &'a str,
    control_type: i32,
    enabled: bool,
    focused: bool,
    offscreen: bool,
    bounds: Option<SemanticBounds>,
}

#[derive(Serialize)]
struct SemanticTree<'a> {
    schema_version: u32,
    scenario: &'a str,
    source: &'static str,
    ordering: &'static str,
    window_title: &'a str,
    nodes: Vec<SemanticNode<'a>>,
}

#[derive(Serialize)]
struct SuiteScenarioSummary<'a> {
    scenario: &'a str,
    directory: &'a str,
}

#[derive(Serialize)]
struct SuiteManifest<'a> {
    schema_version: u32,
    git_sha: &'a str,
    git_dirty: bool,
    scenarios: Vec<SuiteScenarioSummary<'a>>,
}

pub(super) fn run_visual_suite_from_args(args: &[String]) -> Result<String, String> {
    let options = parse_visual_suite_options(args)?;
    let binary_path = resolve_binary_path(&options.binary_path)?;
    fs::create_dir_all(&options.output_dir).map_err(|error| {
        format!(
            "failed to create visual artifact root {}: {error}",
            options.output_dir.display()
        )
    })?;

    let git_sha = git_output(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let git_dirty = git_output(&["status", "--porcelain", "--untracked-files=normal"])
        .is_none_or(|status| !status.is_empty());

    for scenario in &options.scenarios {
        capture_visual_scenario(
            *scenario,
            &binary_path,
            &options.output_dir,
            options.timeout,
            &git_sha,
            git_dirty,
        )?;
    }

    let summaries = options
        .scenarios
        .iter()
        .map(|scenario| SuiteScenarioSummary {
            scenario: scenario.id(),
            directory: scenario.id(),
        })
        .collect();
    let suite_manifest = SuiteManifest {
        schema_version: VISUAL_SCHEMA_VERSION,
        git_sha: &git_sha,
        git_dirty,
        scenarios: summaries,
    };
    write_json(&options.output_dir.join("manifest.json"), &suite_manifest)?;
    serde_json::to_string(&suite_manifest)
        .map_err(|error| format!("failed to render visual suite result: {error}"))
}

fn parse_visual_suite_options(args: &[String]) -> Result<VisualSuiteOptions, String> {
    let mut binary_path = default_binary_path();
    let mut output_dir = PathBuf::from("target").join("gui-visual");
    let mut timeout = Duration::from_millis(20_000);
    let mut scenarios = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--visual-suite" => index += 1,
            "--binary" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--binary requires a path".to_owned())?;
                binary_path = PathBuf::from(value);
                index += 2;
            }
            "--output-dir" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--output-dir requires a path".to_owned())?;
                output_dir = PathBuf::from(value);
                index += 2;
            }
            "--timeout-ms" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--timeout-ms requires an integer value".to_owned())?;
                timeout = parse_timeout_ms(value)?;
                index += 2;
            }
            "--scenario" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--scenario requires a scenario ID".to_owned())?;
                scenarios.push(VisualScenario::parse(value)?);
                index += 2;
            }
            "--help" | "-h" => {
                return Err("usage: sorotte-gui-native-smoke --visual-suite [--binary PATH] [--output-dir PATH] [--timeout-ms N] [--scenario ID]".to_owned());
            }
            argument => return Err(format!("unknown visual suite argument {argument:?}")),
        }
    }
    if scenarios.is_empty() {
        scenarios.extend(VisualScenario::ALL);
    }
    scenarios.dedup();
    Ok(VisualSuiteOptions {
        binary_path,
        output_dir,
        timeout,
        scenarios,
    })
}

fn capture_visual_scenario(
    scenario: VisualScenario,
    binary_path: &Path,
    output_root: &Path,
    timeout: Duration,
    git_sha: &str,
    git_dirty: bool,
) -> Result<(), String> {
    let artifact_dir = output_root.join(scenario.id());
    fs::create_dir_all(&artifact_dir).map_err(|error| {
        format!(
            "failed to create scenario artifact directory {}: {error}",
            artifact_dir.display()
        )
    })?;
    for file_name in ["window.png", "semantic-tree.json", "manifest.json"] {
        let path = artifact_dir.join(file_name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove stale visual artifact {}: {error}",
                    path.display()
                ));
            }
        }
    }

    let runtime_root = std::env::temp_dir().join(format!(
        "sorotte-gui-visual-{}-{}-{}",
        std::process::id(),
        scenario.id().replace('.', "-"),
        unique_suffix()
    ));
    fs::create_dir_all(&runtime_root).map_err(|error| {
        format!(
            "failed to create visual scenario runtime directory {}: {error}",
            runtime_root.display()
        )
    })?;
    let config_path = runtime_root.join("sorotte.ini");
    let media_search_path = runtime_root.join("media-search");
    let open_media_path = runtime_root.join("open-target.mkv");
    fs::create_dir_all(&media_search_path)
        .map_err(|error| format!("failed to create visual media directory: {error}"))?;
    fs::write(&open_media_path, b"visual-open-target")
        .map_err(|error| format!("failed to seed visual media fixture: {error}"))?;
    if !scenario.first_run() {
        seed_native_smoke_config(&config_path)?;
    }
    let mut main_window_entries = vec![("activeView", "setup".to_owned())];
    if !scenario.first_run() {
        main_window_entries.push(("configurationTab", "Connection".to_owned()));
    }
    write_legacy_gui_qsettings_ini(
        &legacy_gui_qsettings_store_path(&runtime_root, "MainWindow"),
        &[("MainWindow", main_window_entries)],
    )?;

    let driver = PlatformNativeGuiDriver;
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_path,
        open_media_file_path: &open_media_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: (!scenario.first_run())
            .then_some((CONFIG_USERNAME_VALUE, CONFIG_ROOM_VALUE)),
        attach_test_player: !scenario.first_run(),
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let launch_result = launch_sorotte_gui_with_retry(&driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&runtime_root);
            return Err(format!("{} launch failed: {error}", scenario.id()));
        }
    };

    let result = (|| {
        thread::sleep(VISUAL_SETTLE_DELAY);
        let mut interaction_target = None;
        if scenario.first_run() {
            wait_for_semantic_name(
                &driver,
                window,
                &["mpv Setup", "Player Setup", "Choose mpv.exe"],
                timeout,
            )?;
        } else {
            wait_for_semantic_name(&driver, window, &["Alpha: alpha.example:8999"], timeout)?;
            let settled_nodes = driver.accessibility_nodes(window)?;
            if settled_nodes
                .iter()
                .any(|node| !node.offscreen && node.name == "ERROR")
            {
                return Err(
                    "configured visual fixture surfaced an unexpected runtime error alert"
                        .to_owned(),
                );
            }

            if scenario == VisualScenario::ConnectionDirty {
                let nodes = driver.accessibility_nodes(window)?;
                let target = select_control_target(&nodes, CONNECTION_HOST_AUTOMATION_ID)?;
                driver.set_named_edit_value(window, &target, "visual-dirty.example", false)?;
                interaction_target = Some(target);
            } else if scenario == VisualScenario::ValidationErrors {
                let nodes = driver.accessibility_nodes(window)?;
                let target = select_control_target(&nodes, CONNECTION_PORT_AUTOMATION_ID)?;
                driver.set_named_edit_value(window, &target, "not-a-port", false)?;
                interaction_target = Some(target);
                wait_for_semantic_name(
                    &driver,
                    window,
                    &["Connection / Port: must be a valid TCP port from 1 to 65535."],
                    timeout,
                )?;
            }
            thread::sleep(Duration::from_millis(500));
        }

        let window_path = artifact_dir.join("window.png");
        driver.capture_window_png(window, &window_path)?;
        validate_window_png(&window_path)?;
        // Native capture expands the DWM frame by its transparent fringe before trimming the PNG.
        // Read UIA after capture so semantic bounds describe the exact rendered frame in window.png.
        let window_title = driver.window_title(window)?;
        let nodes = driver.accessibility_nodes(window)?;
        let semantic_tree = semantic_tree(scenario, &window_title, &nodes);
        let semantic_path = artifact_dir.join("semantic-tree.json");
        write_json(&semantic_path, &semantic_tree)?;

        let focused_element = nodes
            .iter()
            .find(|node| node.focused && !node.automation_id.is_empty())
            .map(|node| node.automation_id.as_str());
        let manifest = ScenarioManifest {
            schema_version: VISUAL_SCHEMA_VERSION,
            scenario: scenario.id(),
            git_sha,
            git_dirty,
            theme: "system",
            viewport: ViewportManifest {
                width: VISUAL_VIEWPORT_WIDTH,
                height: VISUAL_VIEWPORT_HEIGHT,
                scale: "system",
            },
            selected_page: scenario.selected_page(),
            dirty_settings: scenario.dirty_settings(),
            focused_element,
            interaction_target: interaction_target.as_deref(),
            artifacts: ArtifactManifest {
                window: artifact_file_manifest(&window_path, "window.png")?,
                semantics: artifact_file_manifest(&semantic_path, "semantic-tree.json")?,
            },
            determinism: DeterminismManifest {
                viewport: "fixed by native smoke driver",
                configuration: "isolated per-scenario fixture",
                public_servers: "fixed local response fixture",
                theme: "system-following; application exposes no native test override",
                scale: "system-following; application exposes no native test override",
                fonts: "egui system defaults; application exposes no font fixture hook",
            },
        };
        write_json(&artifact_dir.join("manifest.json"), &manifest)
    })();

    close_visual_child(&driver, window, &mut child, timeout);
    let _ = fs::remove_dir_all(&runtime_root);
    result.map_err(|error| format!("{} capture failed: {error}", scenario.id()))
}

fn select_control_target(
    nodes: &[NativeAccessibilityNode],
    automation_id: &str,
) -> Result<String, String> {
    if nodes
        .iter()
        .any(|node| !node.offscreen && node.automation_id == automation_id)
    {
        return Ok(automation_id.to_owned());
    }
    Err(format!(
        "expected stable automation ID {automation_id:?} was not visible",
    ))
}

fn wait_for_semantic_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_names: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let names = driver.accessible_names(window)?;
        if expected_names
            .iter()
            .any(|expected| names.iter().any(|name| name == expected))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for one of [{}]; last accessible names: {}",
                expected_names.join(", "),
                names.join(" | ")
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn semantic_tree<'a>(
    scenario: VisualScenario,
    window_title: &'a str,
    nodes: &'a [NativeAccessibilityNode],
) -> SemanticTree<'a> {
    SemanticTree {
        schema_version: VISUAL_SCHEMA_VERSION,
        scenario: scenario.id(),
        source: "Windows UI Automation",
        ordering: "flat TreeScope_Subtree enumeration",
        window_title,
        nodes: nodes
            .iter()
            .enumerate()
            .map(|(index, node)| SemanticNode {
                index,
                name: &node.name,
                automation_id: &node.automation_id,
                control_type: node.control_type,
                enabled: node.enabled,
                focused: node.focused,
                offscreen: node.offscreen,
                bounds: node.bounds.map(|bounds| SemanticBounds {
                    left: bounds[0],
                    top: bounds[1],
                    right: bounds[2],
                    bottom: bounds[3],
                }),
            })
            .collect(),
    }
}

fn close_visual_child<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    child: &mut Child,
    timeout: Duration,
) {
    let _ = driver.close_window(window);
    if wait_for_process_exit(child, timeout.min(Duration::from_secs(4))).is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut json = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    json.push(b'\n');
    fs::write(path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn artifact_file_manifest(
    path: &Path,
    relative_path: &'static str,
) -> Result<ArtifactFileManifest, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to inspect artifact {}: {error}", path.display()))?;
    Ok(ArtifactFileManifest {
        path: relative_path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}

fn validate_window_png(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to validate screenshot {}: {error}", path.display()))?;
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(format!(
            "native screenshot {} was not a valid PNG artifact",
            path.display()
        ));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width != VISUAL_VIEWPORT_WIDTH as u32 || height != VISUAL_VIEWPORT_HEIGHT as u32 {
        return Err(format!(
            "native screenshot {} had dimensions {width}x{height}; expected {}x{}",
            path.display(),
            VISUAL_VIEWPORT_WIDTH,
            VISUAL_VIEWPORT_HEIGHT
        ));
    }
    Ok(())
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accessibility_node(name: &str, automation_id: &str) -> NativeAccessibilityNode {
        NativeAccessibilityNode {
            name: name.to_owned(),
            automation_id: automation_id.to_owned(),
            control_type: 0,
            enabled: true,
            focused: false,
            offscreen: false,
            bounds: Some([0, 0, 10, 10]),
        }
    }

    #[test]
    fn scenario_parser_accepts_only_declared_artifact_ids() {
        for scenario in VisualScenario::ALL {
            assert_eq!(VisualScenario::parse(scenario.id()), Ok(scenario));
        }
        assert!(VisualScenario::parse("settings.connection.unknown").is_err());
    }

    #[test]
    fn visual_edits_require_stable_automation_ids() {
        let named_only = [accessibility_node("Host", "")];
        assert!(select_control_target(&named_only, CONNECTION_HOST_AUTOMATION_ID).is_err());

        let stable = [accessibility_node(
            "Localized host label",
            CONNECTION_HOST_AUTOMATION_ID,
        )];
        assert_eq!(
            select_control_target(&stable, CONNECTION_HOST_AUTOMATION_ID),
            Ok(CONNECTION_HOST_AUTOMATION_ID.to_owned())
        );
    }
}
