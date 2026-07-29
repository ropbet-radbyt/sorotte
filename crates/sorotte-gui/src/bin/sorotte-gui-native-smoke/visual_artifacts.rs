use std::{
    fmt::Write as _,
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

const VISUAL_SCHEMA_VERSION: u32 = 2;
const VISUAL_SETTLE_DELAY: Duration = Duration::from_millis(800);
const VISUAL_WIDE_VIEWPORT: (i32, i32) = (1700, 1100);
const VISUAL_RECOVERY_VIEWPORT: (i32, i32) = (1700, 1300);
const VISUAL_NARROW_VIEWPORT: (i32, i32) = (900, 760);
const CONNECTION_HOST_AUTOMATION_ID: &str = "settings.connection.host";
const CONNECTION_PORT_AUTOMATION_ID: &str = "settings.connection.port";
const CONNECTION_ROOM_AUTOMATION_ID: &str = "settings.connection.room";
const CONNECTION_PASSWORD_AUTOMATION_ID: &str = "settings.connection.server_password";
const CONNECTION_PASSWORD_CHANGE_AUTOMATION_ID: &str = "settings.connection.server_password.change";
const CONNECTION_PASSWORD_REMOVE_AUTOMATION_ID: &str = "settings.connection.server_password.remove";
const PLAYER_ARGUMENTS_AUTOMATION_ID: &str = "settings.player.arguments";
const CONFIG_SAVE_AUTOMATION_ID: &str = "config-command:save";
const CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID: &str = "config-command:save-and-connect";
const CONFIG_CONNECT_ONCE_AUTOMATION_ID: &str = "config-command:connect-once";
const CONFIG_CLEAR_GUI_DATA_AUTOMATION_ID: &str = "config-command:clear-gui-data";
const CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID: &str = "config-command:confirm-clear-gui-data";
const CONFIG_CANCEL_CLEAR_GUI_DATA_AUTOMATION_ID: &str = "config-command:cancel-clear-gui-data";
const CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID: &str = "configuration:tab:interface-system";
const CONFIG_PLAYBACK_SEARCH_TAB_AUTOMATION_ID: &str = "configuration:tab:playback-search";
const CONFIGURATION_SURFACE_AUTOMATION_ID: &str = "configuration-root";
const MAIN_WINDOW_SURFACE_AUTOMATION_ID: &str = "main-window-root";
const PLUGINS_SURFACE_AUTOMATION_ID: &str = "plugins-root";
const STREAM_SUPPORT_ENABLED_AUTOMATION_ID: &str = "plugins:stream-support:enabled";
const STREAMING_ADVANCED_AUTOMATION_ID: &str = "settings.streaming.recovery_retry_budget";
const STORAGE_BROWSE_AUTOMATION_ID: &str = "config-storage:root:browse";
const PLAYER_SETUP_MODAL_OPEN_SETTINGS_AUTOMATION_ID: &str =
    "shell:modal:player-setup:open-settings";
const PLAYER_SETUP_AUTODETECT_AUTOMATION_ID: &str = "config-player-setup:autodetect";
const PLAYER_SETUP_MODAL_CLOSE_AUTOMATION_ID: &str = "shell:modal:close";
const MAIN_WINDOW_PLAYER_SETUP_RETRY_AUTOMATION_ID: &str = "main-window:player-setup:retry";
const MAIN_WINDOW_PAUSE_AUTOMATION_ID: &str = "main-window:control:pause";
const PLAYER_SETTINGS_DEGRADED_DETAIL: &str = "Playback remains available. Retry mpv settings to apply the remaining streaming options in place.";
const VISUAL_PASSWORD_SEED: &str = "visual-password-seed";
const VISUAL_PASSWORD_REPLACEMENT: &str = "visual-password-replacement";
const SAVE_AND_CONNECT_ROOM: &str = "visual-save-and-connect-room";
const CONNECT_ONCE_ROOM: &str = "visual-connect-once-room";
const CONNECT_ONCE_PLAYER_ARGUMENTS: &str = "--visual-connect-once";
const RECONNECT_HOST: &str = "reconnect-required.example";
const FAILURE_HOST: &str = "save-failure.example";
const PLUGIN_DIRTY_HOST: &str = "plugin-dirty.example";
const SAVE_AND_CONNECT_SERVER_LINES: &[&str] = &[
    r#"{"Hello":{"username":"smoke-user","room":{"name":"visual-save-and-connect-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
];
const CONNECT_ONCE_SERVER_LINES: &[&str] = &[
    r#"{"Hello":{"username":"smoke-user","room":{"name":"visual-connect-once-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisualScenario {
    FirstRunPlayerMissing,
    PlayerSettingsDegraded,
    ConnectionClean,
    ConnectionDirty,
    ValidationErrors,
    SaveAndConnect,
    ConnectOnceDirty,
    ReconnectRequired,
    PasswordConfigured,
    PasswordChange,
    PasswordRemove,
    PersistenceFailure,
    PluginToggleDirty,
    NarrowLight,
    WideDark,
    StreamingAdvanced,
    StorageLocationPending,
    DataDangerZone,
}

impl VisualScenario {
    const ALL: [Self; 18] = [
        Self::FirstRunPlayerMissing,
        Self::PlayerSettingsDegraded,
        Self::ConnectionClean,
        Self::ConnectionDirty,
        Self::ValidationErrors,
        Self::SaveAndConnect,
        Self::ConnectOnceDirty,
        Self::ReconnectRequired,
        Self::PasswordConfigured,
        Self::PasswordChange,
        Self::PasswordRemove,
        Self::PersistenceFailure,
        Self::PluginToggleDirty,
        Self::NarrowLight,
        Self::WideDark,
        Self::StreamingAdvanced,
        Self::StorageLocationPending,
        Self::DataDangerZone,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::FirstRunPlayerMissing => "settings.first-run.player-missing",
            Self::PlayerSettingsDegraded => "room.player-settings-degraded",
            Self::ConnectionClean => "settings.connection.clean",
            Self::ConnectionDirty => "settings.connection.dirty",
            Self::ValidationErrors => "settings.validation-errors",
            Self::SaveAndConnect => "settings.save-and-connect",
            Self::ConnectOnceDirty => "settings.connect-once-dirty",
            Self::ReconnectRequired => "settings.reconnect-required",
            Self::PasswordConfigured => "settings.password.configured",
            Self::PasswordChange => "settings.password.change",
            Self::PasswordRemove => "settings.password.remove",
            Self::PersistenceFailure => "settings.persistence-failure",
            Self::PluginToggleDirty => "plugins.toggle-with-dirty-settings",
            Self::NarrowLight => "settings.narrow-light",
            Self::WideDark => "settings.wide-dark",
            Self::StreamingAdvanced => "settings.streaming-advanced",
            Self::StorageLocationPending => "settings.storage-location-pending",
            Self::DataDangerZone => "settings.data-danger-zone",
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
        match self {
            Self::FirstRunPlayerMissing => "first-run-player-remediation",
            Self::PlayerSettingsDegraded => "room-playback-recovery",
            Self::PluginToggleDirty => "plugins",
            Self::StreamingAdvanced => "playback-search",
            Self::StorageLocationPending | Self::DataDangerZone => "interface-system",
            _ => "connection",
        }
    }

    fn dirty_settings(self) -> &'static [&'static str] {
        match self {
            Self::ConnectionDirty => &["connection.host"],
            Self::ValidationErrors => &["connection.port"],
            Self::ConnectOnceDirty => &[
                "connection.host",
                "connection.port",
                "connection.room",
                "player.arguments",
            ],
            Self::PasswordChange | Self::PasswordRemove => &["connection.server_password"],
            Self::PersistenceFailure | Self::PluginToggleDirty => &["connection.host"],
            _ => &[],
        }
    }

    fn theme(self) -> &'static str {
        if self == Self::WideDark {
            "dark"
        } else {
            "light"
        }
    }

    fn viewport(self) -> (i32, i32) {
        match self {
            Self::NarrowLight => VISUAL_NARROW_VIEWPORT,
            Self::PlayerSettingsDegraded => VISUAL_RECOVERY_VIEWPORT,
            _ => VISUAL_WIDE_VIEWPORT,
        }
    }

    fn configuration_tab(self) -> Option<&'static str> {
        match self {
            Self::FirstRunPlayerMissing
            | Self::PlayerSettingsDegraded
            | Self::PluginToggleDirty => None,
            Self::StreamingAdvanced => Some("playback-search"),
            Self::StorageLocationPending | Self::DataDangerZone => Some("interface-system"),
            _ => Some("connection"),
        }
    }

    fn seeds_password(self) -> bool {
        matches!(
            self,
            Self::PasswordConfigured | Self::PasswordChange | Self::PasswordRemove
        )
    }

    fn uses_default_storage_root(self) -> bool {
        self == Self::StorageLocationPending
    }

    fn uses_loopback_session(self) -> bool {
        !matches!(
            self,
            Self::FirstRunPlayerMissing | Self::SaveAndConnect | Self::ConnectOnceDirty
        )
    }

    fn session_fixture(self) -> &'static str {
        match self {
            Self::FirstRunPlayerMissing => "none",
            Self::SaveAndConnect | Self::ConnectOnceDirty => {
                "reachable local TCP mock on an OS-assigned loopback port"
            }
            _ => "in-process client-core loopback session",
        }
    }

    fn reconnect_required(self) -> bool {
        matches!(
            self,
            Self::ConnectionDirty
                | Self::ValidationErrors
                | Self::ConnectOnceDirty
                | Self::ReconnectRequired
                | Self::PasswordChange
                | Self::PasswordRemove
                | Self::PersistenceFailure
                | Self::PluginToggleDirty
        )
    }

    fn pending_state(self) -> Option<&'static str> {
        match self {
            Self::StorageLocationPending => Some("config-storage-root"),
            Self::PersistenceFailure => Some("save-failed-draft-preserved"),
            Self::DataDangerZone => Some("clear-gui-data-confirmation"),
            _ => None,
        }
    }

    fn validated_semantics(self) -> &'static [&'static str] {
        match self {
            Self::FirstRunPlayerMissing => &[
                PLAYER_SETUP_MODAL_OPEN_SETTINGS_AUTOMATION_ID,
                PLAYER_SETUP_AUTODETECT_AUTOMATION_ID,
            ],
            Self::PlayerSettingsDegraded => &[
                MAIN_WINDOW_PLAYER_SETUP_RETRY_AUTOMATION_ID,
                MAIN_WINDOW_PAUSE_AUTOMATION_ID,
            ],
            Self::ConnectionClean | Self::NarrowLight | Self::WideDark => {
                &[CONNECTION_HOST_AUTOMATION_ID]
            }
            Self::ConnectionDirty => &[CONNECTION_HOST_AUTOMATION_ID, CONFIG_SAVE_AUTOMATION_ID],
            // AccessKit exposes the validation message text, but the field's stable ID is the
            // associated edit control. The scenario interaction separately asserts the exact
            // validation message before capture.
            Self::ValidationErrors => &[CONNECTION_PORT_AUTOMATION_ID],
            Self::SaveAndConnect => &[
                CONNECTION_HOST_AUTOMATION_ID,
                CONNECTION_PORT_AUTOMATION_ID,
                CONNECTION_ROOM_AUTOMATION_ID,
                CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID,
            ],
            Self::ConnectOnceDirty => &[
                CONNECTION_HOST_AUTOMATION_ID,
                CONNECTION_PORT_AUTOMATION_ID,
                CONNECTION_ROOM_AUTOMATION_ID,
                PLAYER_ARGUMENTS_AUTOMATION_ID,
                CONFIG_CONNECT_ONCE_AUTOMATION_ID,
            ],
            Self::ReconnectRequired => &[CONNECTION_HOST_AUTOMATION_ID],
            Self::PasswordConfigured => &[
                CONNECTION_PASSWORD_AUTOMATION_ID,
                CONNECTION_PASSWORD_CHANGE_AUTOMATION_ID,
                CONNECTION_PASSWORD_REMOVE_AUTOMATION_ID,
            ],
            Self::PasswordChange => &[
                CONNECTION_PASSWORD_AUTOMATION_ID,
                CONNECTION_PASSWORD_CHANGE_AUTOMATION_ID,
            ],
            Self::PasswordRemove => &[
                CONNECTION_PASSWORD_AUTOMATION_ID,
                CONNECTION_PASSWORD_REMOVE_AUTOMATION_ID,
            ],
            Self::PersistenceFailure => &[CONNECTION_HOST_AUTOMATION_ID, CONFIG_SAVE_AUTOMATION_ID],
            Self::PluginToggleDirty => &[
                STREAM_SUPPORT_ENABLED_AUTOMATION_ID,
                PLUGINS_SURFACE_AUTOMATION_ID,
            ],
            Self::StreamingAdvanced => &[STREAMING_ADVANCED_AUTOMATION_ID],
            Self::StorageLocationPending => &[STORAGE_BROWSE_AUTOMATION_ID],
            Self::DataDangerZone => &[
                CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID,
                CONFIG_CANCEL_CLEAR_GUI_DATA_AUTOMATION_ID,
            ],
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
    session: &'static str,
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
    reconnect_required: bool,
    pending_state: Option<&'static str>,
    validated_semantics: &'static [&'static str],
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

    let runtime_root = std::env::temp_dir()
        .join("sorotte-gui-visual-fixtures")
        .join(scenario.id().replace('.', "-"));
    match fs::remove_dir_all(&runtime_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to reset visual scenario runtime directory {}: {error}",
                runtime_root.display()
            ));
        }
    }
    fs::create_dir_all(&runtime_root).map_err(|error| {
        format!(
            "failed to create visual scenario runtime directory {}: {error}",
            runtime_root.display()
        )
    })?;
    let config_path = if scenario.uses_default_storage_root() {
        runtime_root.join("Sorotte").join("sorotte.ini")
    } else {
        runtime_root.join("sorotte.ini")
    };
    let media_search_path = runtime_root.join("media-search");
    let open_media_path = runtime_root.join("open-target.mkv");
    let config_storage_browse_path = runtime_root.join("relocated-settings");
    fs::create_dir_all(&media_search_path)
        .map_err(|error| format!("failed to create visual media directory: {error}"))?;
    fs::write(&open_media_path, b"visual-open-target")
        .map_err(|error| format!("failed to seed visual media fixture: {error}"))?;
    seed_visual_scenario_config(scenario, &config_path)?;
    let active_view = if scenario == VisualScenario::PlayerSettingsDegraded {
        "room"
    } else {
        "setup"
    };
    let mut main_window_entries = vec![("activeView", active_view.to_owned())];
    if let Some(configuration_tab) = scenario.configuration_tab() {
        main_window_entries.push(("configurationTab", configuration_tab.to_owned()));
    }
    let qsettings_root = config_path.parent().unwrap_or(&runtime_root);
    write_legacy_gui_qsettings_ini(
        &legacy_gui_qsettings_store_path(qsettings_root, "MainWindow"),
        &[("MainWindow", main_window_entries)],
    )?;

    let mut session_server = match scenario {
        VisualScenario::SaveAndConnect => Some(
            native_smoke_runner::start_visual_mock_session_server(SAVE_AND_CONNECT_SERVER_LINES)
                .map_err(|error| format!("{} server setup failed: {error}", scenario.id()))?,
        ),
        VisualScenario::ConnectOnceDirty => Some(
            native_smoke_runner::start_visual_mock_session_server(CONNECT_ONCE_SERVER_LINES)
                .map_err(|error| format!("{} server setup failed: {error}", scenario.id()))?,
        ),
        _ => None,
    };
    let driver = PlatformNativeGuiDriver;
    let public_servers_spec = if matches!(
        scenario,
        VisualScenario::SaveAndConnect | VisualScenario::ConnectOnceDirty
    ) {
        "[]"
    } else {
        DEFAULT_PUBLIC_SERVERS_SPEC
    };
    let dropped_media_spec = open_media_path.display().to_string();
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_path,
        open_media_file_path: &open_media_path,
        public_servers_spec,
        network_mode: if scenario.uses_loopback_session() {
            NativeNetworkMode::InProcessLoopback {
                username: CONFIG_USERNAME_VALUE,
                room: CONFIG_ROOM_VALUE,
            }
        } else {
            NativeNetworkMode::Detached
        },
        attach_test_player: !scenario.first_run(),
        drop_file_paths_spec: (scenario == VisualScenario::PlayerSettingsDegraded)
            .then_some(dropped_media_spec.as_str()),
        drop_target: (scenario == VisualScenario::PlayerSettingsDegraded).then_some("playlist"),
    };
    let launch_result = launch_sorotte_gui_with_retry_and_test_overrides(
        &driver,
        binary_path,
        launch,
        timeout,
        GuiLaunchTestOverrides {
            theme: Some(scenario.theme()),
            appdata_root: scenario
                .uses_default_storage_root()
                .then_some(runtime_root.as_path()),
            config_storage_browse_path: (scenario == VisualScenario::StorageLocationPending)
                .then_some(config_storage_browse_path.as_path()),
            test_player_observation_path: None,
            lifecycle_observation_path: None,
            disable_startup_saved_connect: matches!(
                scenario,
                VisualScenario::SaveAndConnect | VisualScenario::ConnectOnceDirty
            ),
            player_settings_degraded: scenario == VisualScenario::PlayerSettingsDegraded,
        },
    );
    let (mut child, window) = match launch_result {
        Ok(value) => value,
        Err(error) => {
            if let Some(server) = session_server.take() {
                let _ =
                    native_smoke_runner::release_visual_mock_session_server(server, scenario.id());
            }
            let _ = fs::remove_dir_all(&runtime_root);
            return Err(format!("{} launch failed: {error}", scenario.id()));
        }
    };

    let result = (|| {
        let (viewport_width, viewport_height) = scenario.viewport();
        driver.prepare_window_for_dimensions(window, viewport_width, viewport_height)?;
        thread::sleep(VISUAL_SETTLE_DELAY);
        let interaction_target = if scenario.first_run() {
            wait_for_semantic_name(
                &driver,
                window,
                &["mpv Setup", "Player Setup", "Choose mpv.exe"],
                timeout,
            )?;
            None
        } else {
            let initial_surface_id = if scenario == VisualScenario::PlayerSettingsDegraded {
                MAIN_WINDOW_SURFACE_AUTOMATION_ID
            } else {
                CONFIGURATION_SURFACE_AUTOMATION_ID
            };
            wait_for_visible_semantic_id(
                &driver,
                window,
                initial_surface_id,
                viewport_height,
                timeout,
            )?;
            let settled_nodes = driver.accessibility_nodes(window)?;
            if settled_nodes
                .iter()
                .any(|node| !node.offscreen && node.name == "ERROR")
            {
                let visible_names = settled_nodes
                    .iter()
                    .filter(|node| !node.offscreen && !node.name.is_empty())
                    .map(|node| node.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                return Err(format!(
                    "configured visual fixture surfaced an unexpected runtime error alert: {visible_names}"
                ));
            }

            prepare_visual_scenario_state(
                scenario,
                &driver,
                window,
                &config_path,
                session_server.as_ref(),
                viewport_height,
                timeout,
            )?
        };
        thread::sleep(Duration::from_millis(500));

        let pre_capture_nodes = driver.accessibility_nodes(window)?;
        validate_scenario_semantics(scenario, &pre_capture_nodes)?;
        assert_visual_secrets_redacted(&pre_capture_nodes)?;

        let window_path = artifact_dir.join("window.png");
        driver.capture_window_png(window, &window_path)?;
        validate_window_png(&window_path, viewport_width, viewport_height)?;
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
            theme: scenario.theme(),
            viewport: ViewportManifest {
                width: viewport_width,
                height: viewport_height,
                scale: "system",
            },
            selected_page: scenario.selected_page(),
            dirty_settings: scenario.dirty_settings(),
            reconnect_required: scenario.reconnect_required(),
            pending_state: scenario.pending_state(),
            validated_semantics: scenario.validated_semantics(),
            focused_element,
            interaction_target: interaction_target.as_deref(),
            artifacts: ArtifactManifest {
                window: artifact_file_manifest(&window_path, "window.png")?,
                semantics: artifact_file_manifest(&semantic_path, "semantic-tree.json")?,
            },
            determinism: DeterminismManifest {
                viewport: "fixed per scenario by native smoke driver",
                configuration: "isolated per-scenario fixture",
                public_servers: "fixed local response fixture",
                session: scenario.session_fixture(),
                theme: "forced light/dark by native visual test override",
                scale: "system-following; application exposes no native test override",
                fonts: "egui system defaults; application exposes no font fixture hook",
            },
        };
        write_json(&artifact_dir.join("manifest.json"), &manifest)
    })();

    close_visual_child(&driver, window, &mut child, timeout);
    let server_release_result = session_server.take().map_or(Ok(()), |server| {
        native_smoke_runner::release_visual_mock_session_server(server, scenario.id())
    });
    let _ = fs::remove_dir_all(&runtime_root);
    match (result, server_release_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(format!("{} capture failed: {error}", scenario.id())),
        (Ok(()), Err(error)) => Err(format!("{} server cleanup failed: {error}", scenario.id())),
        (Err(error), Err(release_error)) => Err(format!(
            "{} capture failed: {error}; server cleanup failed: {release_error}",
            scenario.id()
        )),
    }
}

fn seed_visual_scenario_config(scenario: VisualScenario, config_path: &Path) -> Result<(), String> {
    if scenario.first_run() {
        return Ok(());
    }
    if matches!(
        scenario,
        VisualScenario::SaveAndConnect | VisualScenario::ConnectOnceDirty
    ) {
        // Begin genuinely disconnected so these scenarios exercise the complete connection
        // lifecycle instead of reconnecting an already-active loopback session.
        seed_native_smoke_config_with_saved_server(config_path, None, Some(8999))?;
    } else {
        seed_native_smoke_config(config_path)?;
    }
    if !scenario.seeds_password() && scenario != VisualScenario::PluginToggleDirty {
        return Ok(());
    }

    let mut settings = load_sorotte_ini_stored_client_settings_mvp_from_path(config_path)
        .map_err(|error| {
            format!(
                "failed to reload visual config fixture {}: {error}",
                config_path.display()
            )
        })?
        .ok_or_else(|| {
            format!(
                "visual config fixture {} was unexpectedly empty",
                config_path.display()
            )
        })?;
    if scenario.seeds_password() {
        settings.server_password = Some(VISUAL_PASSWORD_SEED.into());
    }
    if scenario == VisualScenario::PluginToggleDirty {
        settings.stream_support_plugin_enabled = Some(true);
    }
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(|error| {
        format!(
            "failed to update visual config fixture {}: {error}",
            config_path.display()
        )
    })
}

fn validate_visual_session_hello(scenario: VisualScenario, hello: &str) -> Result<(), String> {
    let expected_room = match scenario {
        VisualScenario::SaveAndConnect => SAVE_AND_CONNECT_ROOM,
        VisualScenario::ConnectOnceDirty => CONNECT_ONCE_ROOM,
        _ => {
            return Err(format!(
                "scenario {} unexpectedly connected to a visual session server",
                scenario.id()
            ));
        }
    };
    for expected in ["\"Hello\"", CONFIG_USERNAME_VALUE, expected_room] {
        if !hello.contains(expected) {
            return Err(format!(
                "{} local session server received unexpected Hello payload {hello:?}; missing {expected:?}",
                scenario.id()
            ));
        }
    }
    Ok(())
}

fn prepare_visual_scenario_state<D: NativeGuiDriver>(
    scenario: VisualScenario,
    driver: &D,
    window: D::WindowHandle,
    config_path: &Path,
    session_server: Option<&MockSessionServer>,
    viewport_height: i32,
    timeout: Duration,
) -> Result<Option<String>, String> {
    match scenario {
        VisualScenario::FirstRunPlayerMissing => unreachable!("first run is handled by caller"),
        VisualScenario::PlayerSettingsDegraded => {
            for expected in [
                "mpv streaming settings incomplete",
                "mpv is ready, but some streaming settings could not be applied to the active media.",
                PLAYER_SETTINGS_DEGRADED_DETAIL,
                "Retry mpv settings",
            ] {
                wait_for_semantic_name(driver, window, &[expected], timeout)?;
            }
            wait_for_semantic_enabled_state(
                driver,
                window,
                PLAYER_SETUP_MODAL_CLOSE_AUTOMATION_ID,
                true,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                PLAYER_SETUP_MODAL_CLOSE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["view: room"], timeout)?;
            wait_for_semantic_name(driver, window, &["open-target.mkv"], timeout)?;
            wait_for_semantic_name(driver, window, &[PLAYER_SETTINGS_DEGRADED_DETAIL], timeout)?;
            wait_for_semantic_enabled_state(
                driver,
                window,
                MAIN_WINDOW_PLAYER_SETUP_RETRY_AUTOMATION_ID,
                true,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_enabled_state(
                driver,
                window,
                MAIN_WINDOW_PAUSE_AUTOMATION_ID,
                true,
                viewport_height,
                timeout,
            )?;
            Ok(Some(
                MAIN_WINDOW_PLAYER_SETUP_RETRY_AUTOMATION_ID.to_owned(),
            ))
        }
        VisualScenario::ConnectionClean
        | VisualScenario::NarrowLight
        | VisualScenario::WideDark => {
            wait_for_visible_semantic_id(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(None)
        }
        VisualScenario::ConnectionDirty => {
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                "visual-dirty.example",
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_SAVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(Some(CONNECTION_HOST_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::ValidationErrors => {
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_PORT_AUTOMATION_ID,
                "not-a-port",
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(
                driver,
                window,
                &["Connection / Port: must be a valid TCP port from 1 to 65535."],
                timeout,
            )?;
            Ok(Some(CONNECTION_PORT_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::SaveAndConnect => {
            let server = session_server.ok_or_else(|| {
                "Save & Connect visual scenario did not start its local session server".to_owned()
            })?;
            let server_port_number = native_smoke_runner::visual_mock_session_server_port(server);
            let server_port = server_port_number.to_string();
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                "127.0.0.1",
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_PORT_AUTOMATION_ID,
                &server_port,
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_ROOM_AUTOMATION_ID,
                SAVE_AND_CONNECT_ROOM,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            let hello = native_smoke_runner::recv_visual_mock_session_hello(
                server,
                timeout,
                scenario.id(),
            )?;
            validate_visual_session_hello(scenario, &hello)?;
            wait_for_semantic_name(driver, window, &["view: room"], timeout)?;
            wait_for_semantic_name(driver, window, &[SAVE_AND_CONNECT_ROOM], timeout)?;
            wait_for_semantic_name(driver, window, &["pending: (none)"], timeout)?;
            wait_for_semantic_name(driver, window, &["Disconnect Session: enabled"], timeout)?;
            wait_for_semantic_name(driver, window, &["Last Action Error: (none)"], timeout)?;
            wait_for_stored_settings(
                config_path,
                timeout,
                "Save & connect local endpoint and room",
                |settings| {
                    settings.host.as_deref() == Some("127.0.0.1")
                        && settings.port == Some(server_port_number)
                        && settings.room.as_deref() == Some(SAVE_AND_CONNECT_ROOM)
                },
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIGURATION_SURFACE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["view: setup"], timeout)?;
            assert_named_edit_value(driver, window, CONNECTION_HOST_AUTOMATION_ID, "127.0.0.1")?;
            assert_named_edit_value(driver, window, CONNECTION_PORT_AUTOMATION_ID, &server_port)?;
            assert_named_edit_value(
                driver,
                window,
                CONNECTION_ROOM_AUTOMATION_ID,
                SAVE_AND_CONNECT_ROOM,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(Some(CONFIG_SAVE_AND_CONNECT_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::ConnectOnceDirty => {
            let server = session_server.ok_or_else(|| {
                "Connect Once visual scenario did not start its local session server".to_owned()
            })?;
            let server_port_number = native_smoke_runner::visual_mock_session_server_port(server);
            let server_port = server_port_number.to_string();
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                "127.0.0.1",
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_PORT_AUTOMATION_ID,
                &server_port,
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_ROOM_AUTOMATION_ID,
                CONNECT_ONCE_ROOM,
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                PLAYER_ARGUMENTS_AUTOMATION_ID,
                CONNECT_ONCE_PLAYER_ARGUMENTS,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_CONNECT_ONCE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIG_CONNECT_ONCE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            let hello = native_smoke_runner::recv_visual_mock_session_hello(
                server,
                timeout,
                scenario.id(),
            )?;
            validate_visual_session_hello(scenario, &hello)?;
            wait_for_semantic_name(driver, window, &["view: room"], timeout)?;
            wait_for_semantic_name(driver, window, &[CONNECT_ONCE_ROOM], timeout)?;
            wait_for_semantic_name(driver, window, &["pending: (none)"], timeout)?;
            wait_for_semantic_name(driver, window, &["Disconnect Session: enabled"], timeout)?;
            wait_for_semantic_name(driver, window, &["Last Action Error: (none)"], timeout)?;
            let persisted = load_visual_settings(config_path)?;
            if persisted.host.is_some()
                || persisted.port != Some(8999)
                || persisted.room.as_deref() != Some(CONFIG_ROOM_VALUE)
                || persisted.per_player_arguments.is_some()
            {
                return Err(format!(
                    "Connect once changed persisted settings: {persisted:?}"
                ));
            }
            invoke_visible_control(
                driver,
                window,
                CONFIGURATION_SURFACE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["view: setup"], timeout)?;
            assert_named_edit_value(driver, window, CONNECTION_HOST_AUTOMATION_ID, "127.0.0.1")?;
            assert_named_edit_value(driver, window, CONNECTION_PORT_AUTOMATION_ID, &server_port)?;
            assert_named_edit_value(
                driver,
                window,
                CONNECTION_ROOM_AUTOMATION_ID,
                CONNECT_ONCE_ROOM,
            )?;
            assert_named_edit_value(
                driver,
                window,
                PLAYER_ARGUMENTS_AUTOMATION_ID,
                CONNECT_ONCE_PLAYER_ARGUMENTS,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_CONNECT_ONCE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(Some(CONFIG_CONNECT_ONCE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::ReconnectRequired => {
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                RECONNECT_HOST,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["Reconnect required"], timeout)?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_SAVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIG_SAVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_stored_settings(
                config_path,
                timeout,
                "reconnect-required host",
                |settings| settings.host.as_deref() == Some(RECONNECT_HOST),
            )?;
            wait_for_semantic_name_contains(driver, window, "Configuration saved.", timeout)?;
            wait_for_semantic_name_contains(driver, window, "Reconnect required", timeout)?;
            scroll_until_visible_semantic_name_contains(
                driver,
                window,
                "Configuration saved.",
                viewport_height,
                timeout,
            )?;
            Ok(Some(CONFIG_SAVE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::PasswordConfigured => {
            wait_for_semantic_name(driver, window, &["Password is configured."], timeout)?;
            wait_for_semantic_enabled_state(
                driver,
                window,
                CONNECTION_PASSWORD_AUTOMATION_ID,
                false,
                viewport_height,
                timeout,
            )?;
            Ok(None)
        }
        VisualScenario::PasswordChange => {
            invoke_visible_control(
                driver,
                window,
                CONNECTION_PASSWORD_CHANGE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_enabled_state(
                driver,
                window,
                CONNECTION_PASSWORD_AUTOMATION_ID,
                true,
                viewport_height,
                timeout,
            )?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_PASSWORD_AUTOMATION_ID,
                VISUAL_PASSWORD_REPLACEMENT,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["Reconnect required"], timeout)?;
            Ok(Some(CONNECTION_PASSWORD_CHANGE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::PasswordRemove => {
            invoke_visible_control(
                driver,
                window,
                CONNECTION_PASSWORD_REMOVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name(driver, window, &["No password is configured."], timeout)?;
            wait_for_semantic_name(driver, window, &["Reconnect required"], timeout)?;
            Ok(Some(CONNECTION_PASSWORD_REMOVE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::PersistenceFailure => {
            fs::remove_file(config_path).map_err(|error| {
                format!(
                    "failed to replace config fixture with failure directory {}: {error}",
                    config_path.display()
                )
            })?;
            fs::create_dir(config_path).map_err(|error| {
                format!(
                    "failed to create config failure directory {}: {error}",
                    config_path.display()
                )
            })?;
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                FAILURE_HOST,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_SAVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIG_SAVE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name_contains(driver, window, "Configuration save failed", timeout)?;
            scroll_until_visible_semantic_name_contains(
                driver,
                window,
                "Configuration save failed",
                viewport_height,
                timeout,
            )?;
            assert_named_edit_value(driver, window, CONNECTION_HOST_AUTOMATION_ID, FAILURE_HOST)?;
            Ok(Some(CONFIG_SAVE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::PluginToggleDirty => {
            set_visible_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                PLUGIN_DIRTY_HOST,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                PLUGINS_SURFACE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_visible_semantic_id(
                driver,
                window,
                STREAM_SUPPORT_ENABLED_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                STREAM_SUPPORT_ENABLED_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_stored_settings(config_path, timeout, "plugin enablement", |settings| {
                settings.stream_support_plugin_enabled == Some(false)
                    && settings.host.as_deref() == Some(CONFIG_HOST_VALUE)
            })?;
            invoke_visible_control(
                driver,
                window,
                CONFIGURATION_SURFACE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_visible_semantic_id(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            assert_named_edit_value(
                driver,
                window,
                CONNECTION_HOST_AUTOMATION_ID,
                PLUGIN_DIRTY_HOST,
            )?;
            invoke_visible_control(
                driver,
                window,
                PLUGINS_SURFACE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_visible_semantic_id(
                driver,
                window,
                STREAM_SUPPORT_ENABLED_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(Some(STREAM_SUPPORT_ENABLED_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::StreamingAdvanced => {
            invoke_visible_control(
                driver,
                window,
                CONFIG_PLAYBACK_SEARCH_TAB_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                STREAMING_ADVANCED_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            Ok(Some(STREAMING_ADVANCED_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::StorageLocationPending => {
            invoke_visible_control(
                driver,
                window,
                CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                STORAGE_BROWSE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                STORAGE_BROWSE_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name_contains(
                driver,
                window,
                "selected custom root (save to apply)",
                timeout,
            )?;
            Ok(Some(STORAGE_BROWSE_AUTOMATION_ID.to_owned()))
        }
        VisualScenario::DataDangerZone => {
            invoke_visible_control(
                driver,
                window,
                CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_CLEAR_GUI_DATA_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            invoke_visible_control(
                driver,
                window,
                CONFIG_CLEAR_GUI_DATA_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            scroll_until_visible_semantic_id(
                driver,
                window,
                CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID,
                viewport_height,
                timeout,
            )?;
            wait_for_semantic_name_contains(
                driver,
                window,
                "This permanently removes saved settings",
                timeout,
            )?;
            Ok(Some(CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID.to_owned()))
        }
    }
}

fn load_visual_settings(config_path: &Path) -> Result<StoredClientSettingsMvp, String> {
    load_sorotte_ini_stored_client_settings_mvp_from_path(config_path)
        .map_err(|error| {
            format!(
                "failed to read visual config {}: {error}",
                config_path.display()
            )
        })?
        .ok_or_else(|| {
            format!(
                "visual config {} did not contain settings",
                config_path.display()
            )
        })
}

fn wait_for_stored_settings(
    config_path: &Path,
    timeout: Duration,
    expected: &str,
    predicate: impl Fn(&StoredClientSettingsMvp) -> bool,
) -> Result<StoredClientSettingsMvp, String> {
    let deadline = Instant::now() + timeout;
    let last = loop {
        let last = match load_visual_settings(config_path) {
            Ok(settings) => {
                if predicate(&settings) {
                    return Ok(settings);
                }
                format!("{settings:?}")
            }
            Err(error) => error,
        };
        if Instant::now() >= deadline {
            break last;
        }
        thread::sleep(Duration::from_millis(100));
    };
    Err(format!(
        "timed out waiting for {expected} in {}; last state: {last}",
        config_path.display()
    ))
}

fn set_visible_edit_value<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    value: &str,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_visible_semantic_id(driver, window, automation_id, viewport_height, timeout)?;
    driver.set_named_edit_value(window, automation_id, value, false)
}

fn assert_named_edit_value<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = driver.get_named_edit_value(window, automation_id)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "edit {automation_id:?} had value {actual:?}; expected {expected:?}"
        ))
    }
}

fn invoke_visible_control<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_visible_semantic_id(driver, window, automation_id, viewport_height, timeout)?;
    driver.invoke_named_control(window, automation_id, NativeControlKind::Any)
}

fn wait_for_visible_semantic_id<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    viewport_height: i32,
    timeout: Duration,
) -> Result<NativeAccessibilityNode, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let nodes = driver.accessibility_nodes(window)?;
        if let Some(node) = nodes.into_iter().find(|node| {
            semantic_node_is_visible(node, viewport_height) && node.automation_id == automation_id
        }) {
            return Ok(node);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for visible semantic ID {automation_id:?}"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_semantic_enabled_state<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    enabled: bool,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let nodes = driver.accessibility_nodes(window)?;
        if nodes.iter().any(|node| {
            semantic_node_is_visible(node, viewport_height)
                && node.automation_id == automation_id
                && node.enabled == enabled
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for semantic ID {automation_id:?} enabled={enabled}"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn scroll_until_visible_semantic_id<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let nodes = driver.accessibility_nodes(window)?;
        if nodes.iter().any(|node| {
            semantic_node_is_visible(node, viewport_height) && node.automation_id == automation_id
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out scrolling to semantic ID {automation_id:?}"
            ));
        }
        if let Some(anchor) = visual_scroll_anchor(&nodes, viewport_height) {
            driver.scroll_named_control_down(window, anchor, NativeControlKind::Any)?;
        } else {
            driver.scroll_active_view_page_down(window)?;
        }
    }
}

fn scroll_until_visible_semantic_name_contains<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected: &str,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut framing_attempts = 0;
    loop {
        let nodes = driver.accessibility_nodes(window)?;
        if let Some(node) = nodes.iter().find(|node| {
            semantic_node_is_visible(node, viewport_height) && node.name.contains(expected)
        }) {
            if node.bounds.is_some_and(|bounds| bounds[1] < 180) && framing_attempts < 4 {
                if let Some(anchor) = visual_scroll_anchor(&nodes, viewport_height) {
                    driver.scroll_named_control_up(window, anchor, NativeControlKind::Any)?;
                } else {
                    driver.scroll_active_view_page_up(window)?;
                }
                framing_attempts += 1;
                thread::sleep(Duration::from_millis(250));
                continue;
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out scrolling to accessible text containing {expected:?}"
            ));
        }
        if let Some(anchor) = visual_scroll_anchor(&nodes, viewport_height) {
            driver.scroll_named_control_up(window, anchor, NativeControlKind::Any)?;
        } else {
            driver.scroll_active_view_page_up(window)?;
        }
    }
}

fn visual_scroll_anchor(nodes: &[NativeAccessibilityNode], viewport_height: i32) -> Option<&str> {
    nodes
        .iter()
        .filter(|node| {
            semantic_node_is_visible(node, viewport_height)
                && node.enabled
                && !node.automation_id.is_empty()
                && node
                    .bounds
                    .is_some_and(|bounds| bounds[0] >= 180 && bounds[3] <= 1_050)
                && !node.automation_id.starts_with("configuration:tab:")
                && !matches!(
                    node.automation_id.as_str(),
                    CONFIGURATION_SURFACE_AUTOMATION_ID | PLUGINS_SURFACE_AUTOMATION_ID
                )
        })
        .max_by_key(|node| node.bounds.map(|bounds| bounds[1]).unwrap_or_default())
        .map(|node| node.automation_id.as_str())
}

fn validate_scenario_semantics(
    scenario: VisualScenario,
    nodes: &[NativeAccessibilityNode],
) -> Result<(), String> {
    let missing = scenario
        .validated_semantics()
        .iter()
        .copied()
        .filter(|expected| !nodes.iter().any(|node| node.automation_id == *expected))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "scenario {} was missing validated semantic IDs: {}",
            scenario.id(),
            missing.join(", ")
        ))
    }
}

fn semantic_node_is_visible(node: &NativeAccessibilityNode, viewport_height: i32) -> bool {
    !node.offscreen
        && node
            .bounds
            .is_none_or(|bounds| bounds[3] > 0 && bounds[1] < viewport_height)
}

fn assert_visual_secrets_redacted(nodes: &[NativeAccessibilityNode]) -> Result<(), String> {
    for canary in [VISUAL_PASSWORD_SEED, VISUAL_PASSWORD_REPLACEMENT] {
        if nodes
            .iter()
            .any(|node| node.name.contains(canary) || node.automation_id.contains(canary))
        {
            return Err(format!(
                "native semantic tree exposed visual password canary {canary:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn select_control_target(
    nodes: &[NativeAccessibilityNode],
    automation_id: &str,
    viewport_height: i32,
) -> Result<String, String> {
    if nodes.iter().any(|node| {
        semantic_node_is_visible(node, viewport_height) && node.automation_id == automation_id
    }) {
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

fn wait_for_semantic_name_contains<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let names = driver.accessible_names(window)?;
        if names.iter().any(|name| name.contains(expected)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for accessible text containing {expected:?}; last accessible names: {}",
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
    let digest = Sha256::digest(&bytes);
    let mut sha256 = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(sha256, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(ArtifactFileManifest {
        path: relative_path,
        bytes: bytes.len() as u64,
        sha256,
    })
}

fn validate_window_png(
    path: &Path,
    expected_width: i32,
    expected_height: i32,
) -> Result<(), String> {
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
    if width != expected_width as u32 || height != expected_height as u32 {
        return Err(format!(
            "native screenshot {} had dimensions {width}x{height}; expected {}x{}",
            path.display(),
            expected_width,
            expected_height
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
    use std::collections::HashSet;

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
        let mut ids = HashSet::new();
        for scenario in VisualScenario::ALL {
            assert_eq!(VisualScenario::parse(scenario.id()), Ok(scenario));
            assert!(ids.insert(scenario.id()), "scenario IDs must be unique");
        }
        assert!(VisualScenario::parse("settings.connection.unknown").is_err());
        for preserved in [
            "settings.first-run.player-missing",
            "settings.connection.clean",
            "settings.connection.dirty",
            "settings.validation-errors",
        ] {
            assert!(
                ids.contains(preserved),
                "existing visual scenario was removed"
            );
        }
    }

    #[test]
    fn scenario_metadata_reports_real_theme_viewport_page_and_dirty_ids() {
        assert_eq!(VisualScenario::WideDark.theme(), "dark");
        assert_eq!(VisualScenario::WideDark.viewport(), VISUAL_WIDE_VIEWPORT);
        assert_eq!(VisualScenario::NarrowLight.theme(), "light");
        assert_eq!(
            VisualScenario::NarrowLight.viewport(),
            VISUAL_NARROW_VIEWPORT
        );
        assert_eq!(
            VisualScenario::PlayerSettingsDegraded.viewport(),
            VISUAL_RECOVERY_VIEWPORT
        );
        assert_eq!(
            VisualScenario::StreamingAdvanced.selected_page(),
            "playback-search"
        );
        assert_eq!(VisualScenario::PluginToggleDirty.selected_page(), "plugins");
        assert_eq!(
            VisualScenario::ConnectOnceDirty.dirty_settings(),
            &[
                "connection.host",
                "connection.port",
                "connection.room",
                "player.arguments",
            ]
        );
        assert!(VisualScenario::ConnectionDirty.reconnect_required());
        assert!(VisualScenario::ConnectOnceDirty.reconnect_required());
        assert!(VisualScenario::ReconnectRequired.reconnect_required());
        assert!(!VisualScenario::SaveAndConnect.reconnect_required());
        assert_eq!(
            VisualScenario::StorageLocationPending.pending_state(),
            Some("config-storage-root")
        );
        assert_eq!(
            VisualScenario::DataDangerZone.pending_state(),
            Some("clear-gui-data-confirmation")
        );
        assert!(!VisualScenario::FirstRunPlayerMissing.uses_loopback_session());
        assert!(!VisualScenario::SaveAndConnect.uses_loopback_session());
        assert!(!VisualScenario::ConnectOnceDirty.uses_loopback_session());
        assert!(VisualScenario::ConnectionClean.uses_loopback_session());
        assert!(VisualScenario::ReconnectRequired.uses_loopback_session());
        assert!(VisualScenario::PlayerSettingsDegraded.uses_loopback_session());
        assert_eq!(
            VisualScenario::PlayerSettingsDegraded.selected_page(),
            "room-playback-recovery"
        );
    }

    #[test]
    fn scenario_manifest_serializes_review_state_and_artifacts() {
        let manifest = ScenarioManifest {
            schema_version: VISUAL_SCHEMA_VERSION,
            scenario: VisualScenario::PluginToggleDirty.id(),
            git_sha: "abc123",
            git_dirty: true,
            theme: VisualScenario::PluginToggleDirty.theme(),
            viewport: ViewportManifest {
                width: VISUAL_WIDE_VIEWPORT.0,
                height: VISUAL_WIDE_VIEWPORT.1,
                scale: "system",
            },
            selected_page: VisualScenario::PluginToggleDirty.selected_page(),
            dirty_settings: VisualScenario::PluginToggleDirty.dirty_settings(),
            reconnect_required: false,
            pending_state: None,
            validated_semantics: VisualScenario::PluginToggleDirty.validated_semantics(),
            focused_element: Some(CONNECTION_HOST_AUTOMATION_ID),
            interaction_target: Some(STREAM_SUPPORT_ENABLED_AUTOMATION_ID),
            artifacts: ArtifactManifest {
                window: ArtifactFileManifest {
                    path: "window.png",
                    bytes: 10,
                    sha256: "window-sha".to_owned(),
                },
                semantics: ArtifactFileManifest {
                    path: "semantic-tree.json",
                    bytes: 20,
                    sha256: "semantic-sha".to_owned(),
                },
            },
            determinism: DeterminismManifest {
                viewport: "fixed",
                configuration: "isolated",
                public_servers: "fixed",
                session: "loopback",
                theme: "forced",
                scale: "system",
                fonts: "system",
            },
        };
        let json = serde_json::to_value(manifest).expect("manifest should serialize");
        assert_eq!(json["schema_version"], VISUAL_SCHEMA_VERSION);
        assert_eq!(json["theme"], "light");
        assert_eq!(json["selected_page"], "plugins");
        assert_eq!(json["dirty_settings"][0], "connection.host");
        assert_eq!(json["determinism"]["session"], "loopback");
        assert_eq!(
            json["validated_semantics"][0],
            STREAM_SUPPORT_ENABLED_AUTOMATION_ID
        );
        assert_eq!(json["artifacts"]["window"]["path"], "window.png");
    }

    #[test]
    fn visual_edits_require_stable_automation_ids() {
        let named_only = [accessibility_node("Host", "")];
        assert!(
            select_control_target(
                &named_only,
                CONNECTION_HOST_AUTOMATION_ID,
                VISUAL_WIDE_VIEWPORT.1,
            )
            .is_err()
        );

        let stable = [accessibility_node(
            "Localized host label",
            CONNECTION_HOST_AUTOMATION_ID,
        )];
        assert_eq!(
            select_control_target(
                &stable,
                CONNECTION_HOST_AUTOMATION_ID,
                VISUAL_WIDE_VIEWPORT.1,
            ),
            Ok(CONNECTION_HOST_AUTOMATION_ID.to_owned())
        );
    }

    #[test]
    fn semantic_visibility_respects_each_scenario_viewport_height() {
        let mut node = accessibility_node("Viewport boundary", "viewport-boundary");
        node.bounds = Some([0, 900, 10, 10]);
        assert!(!semantic_node_is_visible(&node, VISUAL_NARROW_VIEWPORT.1));
        assert!(semantic_node_is_visible(&node, VISUAL_WIDE_VIEWPORT.1));
        assert!(semantic_node_is_visible(&node, VISUAL_RECOVERY_VIEWPORT.1));

        node.bounds = Some([0, 1_200, 10, 10]);
        assert!(!semantic_node_is_visible(&node, VISUAL_NARROW_VIEWPORT.1));
        assert!(!semantic_node_is_visible(&node, VISUAL_WIDE_VIEWPORT.1));
        assert!(semantic_node_is_visible(&node, VISUAL_RECOVERY_VIEWPORT.1));
    }

    #[test]
    fn semantic_validation_requires_declared_ids_and_rejects_secret_canaries() {
        let clean = VisualScenario::PasswordConfigured
            .validated_semantics()
            .iter()
            .map(|automation_id| accessibility_node("Password control", automation_id))
            .collect::<Vec<_>>();
        assert!(validate_scenario_semantics(VisualScenario::PasswordConfigured, &clean).is_ok());
        assert!(assert_visual_secrets_redacted(&clean).is_ok());

        let leaked = [accessibility_node(
            VISUAL_PASSWORD_SEED,
            CONNECTION_PASSWORD_AUTOMATION_ID,
        )];
        assert!(assert_visual_secrets_redacted(&leaked).is_err());
        assert!(validate_scenario_semantics(VisualScenario::PasswordConfigured, &[]).is_err());
    }
}
