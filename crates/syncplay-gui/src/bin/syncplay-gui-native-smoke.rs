use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use syncplay_client_app::{
    legacy_ini_serde::format_serialized_public_servers_list_legacy_compatible,
    legacy_settings::AutoplayThresholdOverride,
    legacy_settings::StoredClientSettingsMvp,
    legacy_syncplay_ini::{
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    },
};
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};
use syncplay_compat::LegacyServerPythonPeerHarness;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct NativeSmokeOptions {
    binary_path: Option<PathBuf>,
    timeout: Duration,
    format: OutputFormat,
    keep_open: bool,
    scenario_filters: Vec<String>,
}

struct NativeSmokeReport {
    binary_path: String,
    pid: u32,
    window_title: String,
    menu_labels: Vec<String>,
    menu_contract: String,
    accessible_name_count: usize,
    accessibility_contract: String,
    interaction_steps: Vec<String>,
    interaction_contract: String,
    duration_ms: u128,
    closed: bool,
}

#[derive(Clone, Copy)]
struct TcpSessionBootstrap<'a> {
    host: &'a str,
    port: u16,
    username: &'a str,
    room: &'a str,
}

#[derive(Clone, Copy)]
struct GuiLaunchConfig<'a> {
    config_path: &'a Path,
    media_search_browse_path: &'a Path,
    open_media_file_path: &'a Path,
    public_servers_spec: &'a str,
    tcp_session: Option<TcpSessionBootstrap<'a>>,
    loopback_session: Option<(&'a str, &'a str)>,
    attach_test_player: bool,
    drop_file_paths_spec: Option<&'a str>,
    drop_target: Option<&'a str>,
}

struct MockSessionServer {
    address: String,
    port: u16,
    hello_rx: mpsc::Receiver<String>,
    release_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

#[path = "syncplay-gui-native-smoke/native_smoke_runner.rs"]
mod native_smoke_runner;
const DEFAULT_PUBLIC_SERVERS_SPEC: &str =
    "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]";
const DLL_INIT_FAILED_STATUS: u32 = 0xC000_0142;
const LAUNCH_ATTEMPTS: usize = 2;
const TRANSPORT_SESSION_USERNAME: &str = "smoke-user";
const TRANSPORT_SESSION_ROOM: &str = "smoke-room";
const LIVE_PYTHON_INTEROP_LOCAL_USERNAME: &str = "interop-gui-user";
const LIVE_PYTHON_INTEROP_PEER_USERNAME: &str = "interop-py-peer";
const LIVE_PYTHON_INTEROP_ROOM: &str = "interop-room";
const LIVE_PYTHON_INTEROP_ALT_ROOM: &str = "interop-room-b";
const LIVE_PYTHON_INTEROP_CONTROLLED_ROOM: &str = "+interop-room:447CE7E3548D";
const LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT: &str = "+interop-room:447CE7E3548D:AB-123-456";
const LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE: &str = "hello from python";
const LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE: &str = "hello again from python";
const LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE: &str = "gui-playlist-1.mkv";
const LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO: &str = "gui-playlist-2.mkv";
const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE: &str = "python-playlist-1.mkv";
const LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO: &str = "python-playlist-2.mkv";
const LIVE_PYTHON_INTEROP_LOCAL_ROW_NAME: &str = "self=yes, ready=no, controller=no";
const LIVE_PYTHON_INTEROP_LOCAL_CONTROLLER_ROW_NAME: &str = "self=yes, ready=no, controller=yes";
const LIVE_PYTHON_INTEROP_LOCAL_READY_ROW_NAME: &str = "self=yes, ready=yes, controller=no";
const LIVE_PYTHON_INTEROP_PEER_ROW_NAME: &str = "self=no, ready=no, controller=no";
const LIVE_PYTHON_INTEROP_PEER_READY_ROW_NAME: &str = "self=no, ready=yes, controller=no";
const MAIN_WINDOW_ROOM_BROWSER_NAME: &str = "Room Browser";
const MAIN_WINDOW_CONTROLS_CONTAINER_NAME: &str = "Controls";
const MAIN_WINDOW_LOCAL_READY_BUTTON_NAME: &str = "Set Ready";
const MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID: &str = "main-window:control:set-ready";
const MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS: usize = 6;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_X: i32 = 32;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_Y: i32 = 32;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_WIDTH: i32 = 1700;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_HEIGHT: i32 = 1100;
const CONFIG_HOST_VALUE: &str = "syncplay.example";
const CONFIG_PORT_VALUE: &str = "8999";
const CONFIG_USERNAME_VALUE: &str = "smoke-user";
const CONFIG_ROOM_VALUE: &str = "smoke-room";
const CONFIG_PLAYER_PATH_VALUE: &str = "C:\\Windows\\System32\\notepad.exe";
const TRUSTED_DOMAINS_EDIT_INDEX: usize = 6;
const TRUSTED_DOMAINS_VALUE: &str = "youtube.com; *.example.com/videos";
const CONFIG_REWIND_THRESHOLD_VALUE: &str = "1.25";
const CONFIG_FASTFORWARD_THRESHOLD_VALUE: &str = "3.5";
const CONFIG_SLOWDOWN_THRESHOLD_VALUE: &str = "2.25";
const CONFIG_CHAT_MAX_LINES_VALUE: &str = "7";
const CONFIG_CHAT_INPUT_FONT_VALUE: &str = "Consolas";
const CONFIG_CHAT_OUTPUT_FONT_VALUE: &str = "Cascadia Mono";
const CONFIG_LANGUAGE_VALUE: &str = "pt_BR";
const CUSTOM_SERVER_LABEL: &str = "Custom";
const CUSTOM_SERVER_HOST: &str = "custom.example";
const CUSTOM_SERVER_PORT: &str = "9001";
const CUSTOM_SERVER_ADDRESS: &str = "custom.example:9001";
const CUSTOM_SERVER_ROW_NAME: &str = "Custom: custom.example:9001";
const MIGRATION_INI_SERVER_LABEL: &str = "Saved";
const MIGRATION_INI_SERVER_HOST: &str = "saved.example";
const MIGRATION_INI_SERVER_PORT: &str = "8999";
const MIGRATION_INI_SERVER_ADDRESS: &str = "saved.example:8999";
const MIGRATION_INI_SERVER_ROW_NAME: &str = "Saved: saved.example:8999";
const MIGRATION_GUI_SERVER_LABEL: &str = "GuiOnly";
const MIGRATION_GUI_SERVER_ADDRESS: &str = "gui-only.example:9002";
const MIGRATION_GUI_SERVER_ROW_NAME: &str = "GuiOnly: gui-only.example:9002";
const MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS: f64 = 3.0;
const MEDIA_SEARCH_TIMEOUT_SECONDS: f64 = 30.0;
const MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS: f64 = 2.5;
const MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS: f64 = 7.5;
#[path = "syncplay-gui-native-smoke/platform_driver.rs"]
mod platform_driver;
use native_smoke_runner::run_native_smoke;
use platform_driver::{NativeControlKind, NativeGuiDriver, PlatformNativeGuiDriver};
impl NativeSmokeReport {
    fn render_text(&self) -> String {
        format!(
            "result=ok\nbinary={}\npid={}\nwindow_title={}\nmenu_labels={}\nmenu_contract={}\naccessible_name_count={}\naccessibility_contract={}\ninteraction_steps={}\ninteraction_contract={}\nclosed={}\nduration_ms={}\n",
            self.binary_path,
            self.pid,
            self.window_title,
            self.menu_labels.join("|"),
            self.menu_contract,
            self.accessible_name_count,
            self.accessibility_contract,
            self.interaction_steps.join("|"),
            self.interaction_contract,
            self.closed,
            self.duration_ms
        )
    }

    fn render_json(&self) -> String {
        let labels = self
            .menu_labels
            .iter()
            .map(|label| render_json_string(label))
            .collect::<Vec<_>>()
            .join(",");
        let interaction_steps = self
            .interaction_steps
            .iter()
            .map(|step| render_json_string(step))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"result\":\"ok\",\"binary\":{},\"pid\":{},\"window_title\":{},\"menu_labels\":[{}],\"menu_contract\":{},\"accessible_name_count\":{},\"accessibility_contract\":{},\"interaction_steps\":[{}],\"interaction_contract\":{},\"closed\":{},\"duration_ms\":{}}}\n",
            render_json_string(&self.binary_path),
            self.pid,
            render_json_string(&self.window_title),
            labels,
            render_json_string(&self.menu_contract),
            self.accessible_name_count,
            render_json_string(&self.accessibility_contract),
            interaction_steps,
            render_json_string(&self.interaction_contract),
            if self.closed { "true" } else { "false" },
            self.duration_ms
        )
    }

    fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.render_text(),
            OutputFormat::Json => self.render_json(),
        }
    }
}

fn render_json_string(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => rendered.push_str("\\\\"),
            '"' => rendered.push_str("\\\""),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ => rendered.push(ch),
        }
    }
    rendered.push('"');
    rendered
}

fn render_error(error: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("result=error\nerror={error}\n"),
        OutputFormat::Json => {
            format!(
                "{{\"result\":\"error\",\"error\":{}}}\n",
                render_json_string(error)
            )
        }
    }
}

fn parse_timeout_ms(token: &str) -> Result<Duration, String> {
    let timeout_ms = token
        .parse::<u64>()
        .map_err(|_| format!("--timeout-ms requires a positive integer, got {token:?}"))?;
    if timeout_ms == 0 {
        return Err("--timeout-ms must be greater than zero".to_owned());
    }
    Ok(Duration::from_millis(timeout_ms))
}

fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn scenario_selected(options: &NativeSmokeOptions, scenario: &str) -> bool {
    options.scenario_filters.is_empty()
        || options
            .scenario_filters
            .iter()
            .any(|candidate| candidate == scenario)
}

fn parse_options(args: &[String]) -> Result<NativeSmokeOptions, String> {
    let mut options = NativeSmokeOptions {
        binary_path: None,
        timeout: Duration::from_millis(10_000),
        format: OutputFormat::Text,
        keep_open: false,
        scenario_filters: Vec::new(),
    };

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--binary" => {
                if index + 1 >= args.len() {
                    return Err("--binary requires a path".to_owned());
                }
                options.binary_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--timeout-ms" => {
                if index + 1 >= args.len() {
                    return Err("--timeout-ms requires an integer value".to_owned());
                }
                options.timeout = parse_timeout_ms(&args[index + 1])?;
                index += 2;
            }
            "--json" => {
                options.format = OutputFormat::Json;
                index += 1;
            }
            "--text" => {
                options.format = OutputFormat::Text;
                index += 1;
            }
            "--keep-open" => {
                options.keep_open = true;
                index += 1;
            }
            "--scenario" => {
                if index + 1 >= args.len() {
                    return Err("--scenario requires a scenario name".to_owned());
                }
                options
                    .scenario_filters
                    .push(args[index + 1].to_ascii_lowercase());
                index += 2;
            }
            "--help" | "-h" => {
                return Err(native_smoke_usage().to_owned());
            }
            argument => {
                return Err(format!("unknown argument {argument:?}"));
            }
        }
    }

    Ok(options)
}

fn native_smoke_usage() -> &'static str {
    "usage: syncplay-gui-native-smoke [--binary PATH] [--timeout-ms N] [--json|--text] [--keep-open] [--scenario NAME]"
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn default_binary_path() -> PathBuf {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let candidate = parent.join("syncplay-gui.exe");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("target")
        .join("debug")
        .join("syncplay-gui.exe")
}

fn resolve_binary_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve syncplay-gui binary at {path:?}: {error}"))
}

fn launch_syncplay_gui(binary_path: &Path, launch: GuiLaunchConfig<'_>) -> Result<Child, String> {
    let mut command = Command::new(binary_path);
    if let Some(parent) = binary_path.parent() {
        command.current_dir(parent);
    }
    for name in [
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP",
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK",
        "SYNCPLAY_GUI_ENABLE_TEST_PLAYER",
        "SYNCPLAY_CLIENT_HOST",
        "SYNCPLAY_CLIENT_PORT",
        "SYNCPLAY_CLIENT_USERNAME",
        "SYNCPLAY_CLIENT_ROOM",
        "SYNCPLAY_CLIENT_MPV_IPC_PATH",
        "SYNCPLAY_MPV_IPC_PATH",
        "SYNCPLAY_GUI_TEST_DROP_FILE_PATHS",
        "SYNCPLAY_GUI_TEST_DROP_TARGET",
    ] {
        command.env_remove(name);
    }
    command.env("SYNCPLAY_CLIENT_CONFIG_PATH", launch.config_path);
    command.env(
        "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS",
        launch.public_servers_spec,
    );
    command.env(
        "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS",
        launch.open_media_file_path.display().to_string(),
    );
    command.env(
        "SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH",
        launch.media_search_browse_path.display().to_string(),
    );
    if let Some(drop_file_paths_spec) = launch.drop_file_paths_spec {
        command.env("SYNCPLAY_GUI_TEST_DROP_FILE_PATHS", drop_file_paths_spec);
    }
    if let Some(drop_target) = launch.drop_target {
        command.env("SYNCPLAY_GUI_TEST_DROP_TARGET", drop_target);
    }
    if let Some(tcp_session) = launch.tcp_session {
        command.env("SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP", "true");
        command.env("SYNCPLAY_CLIENT_HOST", tcp_session.host);
        command.env("SYNCPLAY_CLIENT_PORT", tcp_session.port.to_string());
        command.env("SYNCPLAY_CLIENT_USERNAME", tcp_session.username);
        command.env("SYNCPLAY_CLIENT_ROOM", tcp_session.room);
    } else if let Some((username, room)) = launch.loopback_session {
        command.env("SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK", "true");
        command.env("SYNCPLAY_CLIENT_USERNAME", username);
        command.env("SYNCPLAY_CLIENT_ROOM", room);
    }
    if launch.attach_test_player {
        command.env("SYNCPLAY_GUI_ENABLE_TEST_PLAYER", "true");
    }
    command
        .spawn()
        .map_err(|error| format!("failed to launch syncplay-gui at {binary_path:?}: {error}"))
}

fn wait_for_main_window<D: NativeGuiDriver>(
    driver: &D,
    child: &mut Child,
    timeout: Duration,
) -> Result<D::WindowHandle, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll syncplay-gui process state: {error}"))?
        {
            return Err(format!(
                "syncplay-gui exited before exposing a main window (status: {status})"
            ));
        }

        if let Some(window) = driver.find_main_window(child.id())? {
            return Ok(window);
        }

        if Instant::now() >= deadline {
            return Err("timed out waiting for the syncplay-gui main window".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll syncplay-gui exit state: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                "timed out waiting for syncplay-gui to exit after close request".to_owned(),
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn seed_native_smoke_config_with_saved_server(
    config_path: &Path,
    host: Option<&str>,
    port: Option<u16>,
) -> Result<(), String> {
    let settings = StoredClientSettingsMvp {
        host: host.map(str::to_owned),
        port,
        username: Some(CONFIG_USERNAME_VALUE.to_owned()),
        room: Some(CONFIG_ROOM_VALUE.to_owned()),
        player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
        folder_search_first_file_timeout_seconds: Some(MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS),
        folder_search_timeout_seconds: Some(MEDIA_SEARCH_TIMEOUT_SECONDS),
        folder_search_double_check_interval_seconds: Some(
            MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS,
        ),
        folder_search_warning_threshold_seconds: Some(MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS),
        ..StoredClientSettingsMvp::default()
    };
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(
        |error| {
            format!(
                "failed to seed native smoke config {}: {error}",
                config_path.display()
            )
        },
    )
}

fn seed_native_smoke_config(config_path: &Path) -> Result<(), String> {
    seed_native_smoke_config_with_saved_server(
        config_path,
        Some(CONFIG_HOST_VALUE),
        Some(CONFIG_PORT_VALUE.parse().unwrap()),
    )
}

fn legacy_gui_qsettings_store_path(root: &Path, store_name: &str) -> PathBuf {
    root.join("Syncplay").join(format!("{store_name}.ini"))
}

fn write_legacy_gui_qsettings_ini(
    path: &Path,
    sections: &[(&str, Vec<(&str, String)>)],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create native smoke legacy GUI store directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut contents = String::new();
    for (section, entries) in sections {
        if entries.is_empty() {
            continue;
        }
        contents.push('[');
        contents.push_str(section);
        contents.push_str("]\n");
        for (key, value) in entries {
            contents.push_str(key);
            contents.push_str(" = ");
            contents.push_str(&value.replace('%', "%%"));
            contents.push('\n');
        }
        contents.push('\n');
    }
    fs::write(path, contents).map_err(|error| {
        format!(
            "failed to write native smoke legacy GUI store {}: {error}",
            path.display()
        )
    })
}

fn seed_native_smoke_gui_state(
    root: &Path,
    active_view: Option<&str>,
    selected_public_server_address: Option<&str>,
    public_servers: &[(String, String)],
    last_media_dialog_directory: Option<&Path>,
) -> Result<(), String> {
    let main_window_entries = [
        active_view.map(|value| ("activeView", value.to_owned())),
        selected_public_server_address
            .map(|value| ("selectedPublicServerAddress", value.to_owned())),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !main_window_entries.is_empty() {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "MainWindow"),
            &[("MainWindow", main_window_entries)],
        )?;
    }
    if !public_servers.is_empty() {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "Interface"),
            &[(
                "PublicServerList",
                vec![(
                    "publicServers",
                    format_serialized_public_servers_list_legacy_compatible(public_servers),
                )],
            )],
        )?;
    }
    if let Some(directory) = last_media_dialog_directory {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "MediaBrowseDialog"),
            &[(
                "MediaBrowseDialog",
                vec![("mediadir", directory.display().to_string())],
            )],
        )?;
    }
    Ok(())
}

fn launch_syncplay_gui_with_retry<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    timeout: Duration,
) -> Result<(Child, D::WindowHandle), String> {
    let mut last_error = String::new();
    for attempt in 1..=LAUNCH_ATTEMPTS {
        let mut child = launch_syncplay_gui(binary_path, launch)?;
        match wait_for_main_window(driver, &mut child, timeout) {
            Ok(window) => {
                let _ = driver.prepare_window_for_smoke(window);
                return Ok((child, window));
            }
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

fn wait_for_file_contains(
    path: &Path,
    expected_snippets: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_contents = String::new();
    loop {
        match fs::read_to_string(path) {
            Ok(contents) => {
                if expected_snippets
                    .iter()
                    .all(|snippet| contents.contains(snippet))
                {
                    return Ok(());
                }
                last_contents = contents;
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for config file {path:?} to contain required lines; last read error: {error}"
                    ));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for config file {:?} to contain [{}]. Last file contents:\n{}",
                path,
                expected_snippets
                    .iter()
                    .map(|snippet| format!("{snippet:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                last_contents
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn saved_configuration_mismatch_message(
    settings: &StoredClientSettingsMvp,
    media_search_directory: &str,
) -> Option<String> {
    let expected = expected_saved_configuration(media_search_directory);

    let mut normalized = settings.clone();
    if normalized
        .last_checked_for_updates
        .as_deref()
        .is_some_and(looks_like_legacy_update_timestamp)
    {
        normalized.last_checked_for_updates = None;
    }

    if normalized == expected {
        None
    } else {
        Some(format!("expected {:?}, got {:?}", expected, settings,))
    }
}

fn looks_like_legacy_update_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 23
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b' '
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            _ => byte.is_ascii_digit(),
        })
}

fn expected_saved_configuration(media_search_directory: &str) -> StoredClientSettingsMvp {
    StoredClientSettingsMvp {
        host: Some(CONFIG_HOST_VALUE.to_owned()),
        port: Some(CONFIG_PORT_VALUE.parse().unwrap()),
        username: Some(CONFIG_USERNAME_VALUE.to_owned()),
        room: Some(CONFIG_ROOM_VALUE.to_owned()),
        player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:9000".to_owned()),
        ]),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(true),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(true),
        unpause_action: Some(UnpauseActionMode::Always),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
        filename_privacy_mode: Some(PrivacyMode::SendHashed),
        filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(true),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(true),
        rewind_threshold_seconds: Some(CONFIG_REWIND_THRESHOLD_VALUE.parse().unwrap()),
        fastforward_threshold_seconds: Some(CONFIG_FASTFORWARD_THRESHOLD_VALUE.parse().unwrap()),
        slowdown_threshold_seconds: Some(CONFIG_SLOWDOWN_THRESHOLD_VALUE.parse().unwrap()),
        media_search_directories: Some(vec![media_search_directory.to_owned()]),
        folder_search_first_file_timeout_seconds: Some(MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS),
        folder_search_timeout_seconds: Some(MEDIA_SEARCH_TIMEOUT_SECONDS),
        folder_search_double_check_interval_seconds: Some(
            MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS,
        ),
        folder_search_warning_threshold_seconds: Some(MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(true),
        chat_move_osd: Some(true),
        chat_max_lines: Some(CONFIG_CHAT_MAX_LINES_VALUE.parse().unwrap()),
        chat_input_font_family: Some(CONFIG_CHAT_INPUT_FONT_VALUE.to_owned()),
        chat_output_font_family: Some(CONFIG_CHAT_OUTPUT_FONT_VALUE.to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(true),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(true),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some(CONFIG_LANGUAGE_VALUE.to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    }
}

fn wait_for_saved_configuration(
    config_path: &Path,
    media_search_directory: &str,
    timeout: Duration,
) -> Result<StoredClientSettingsMvp, String> {
    let deadline = Instant::now() + timeout;
    let mut last_contents = String::new();

    let last_mismatch = loop {
        let mismatch = match fs::read_to_string(config_path) {
            Ok(contents) => {
                last_contents = contents;
                match load_syncplay_ini_stored_client_settings_mvp_from_path(config_path) {
                    Ok(Some(settings)) => {
                        if let Some(mismatch) =
                            saved_configuration_mismatch_message(&settings, media_search_directory)
                        {
                            mismatch
                        } else {
                            return Ok(settings);
                        }
                    }
                    Ok(None) => "config file parsed successfully but did not contain any settings"
                        .to_owned(),
                    Err(error) => format!("config file parse failed: {error}"),
                }
            }
            Err(error) => format!("config file read failed: {error}"),
        };

        if Instant::now() >= deadline {
            break mismatch;
        }
        thread::sleep(Duration::from_millis(50));
    };

    Err(format!(
        "timed out waiting for configuration {} to match the expected first-run save contract; last mismatch: {}; last file contents:\n{}",
        config_path.display(),
        last_mismatch,
        last_contents,
    ))
}

fn normalize_menu_label(raw_label: &str) -> String {
    raw_label.replace('&', "").trim().to_owned()
}

fn verify_menu_contract(menu_labels: &[String]) -> Result<(), String> {
    let normalized = menu_labels
        .iter()
        .map(|label| normalize_menu_label(label))
        .collect::<Vec<_>>();
    let required = ["File", "Playback", "Advanced", "Window", "Help"];
    for expected in required {
        if !normalized.iter().any(|label| label == expected) {
            return Err(format!(
                "main window menu is missing required top-level entry {expected:?}; observed: {}",
                normalized.join(", ")
            ));
        }
    }
    Ok(())
}

fn verify_accessibility_contract(accessible_names: &[String]) -> Result<(), String> {
    if accessible_names.is_empty() {
        return Err("accessibility tree did not expose any named elements".to_owned());
    }

    let required_labels = ["File", "Playback", "Advanced", "Window", "Help"];
    for required_label in required_labels {
        if !accessible_names.iter().any(|name| name == required_label) {
            return Err(format!(
                "accessibility tree is missing required top-level label {required_label:?}"
            ));
        }
    }

    if !accessible_names
        .iter()
        .any(|name| name == "view: configuration" || name == "view: main-window")
    {
        return Err(
            "accessibility tree is missing a known view indicator (expected 'view: configuration' or 'view: main-window')"
                .to_owned(),
        );
    }

    Ok(())
}

fn contains_accessible_name(accessible_names: &[String], expected: &str) -> bool {
    accessible_names.iter().any(|name| name == expected)
}

fn contains_accessible_name_fragment(accessible_names: &[String], expected_fragment: &str) -> bool {
    accessible_names
        .iter()
        .any(|name| name.contains(expected_fragment))
}

fn render_accessible_name_snapshot_for_patterns(
    accessible_names: &[String],
    patterns: &[&str],
) -> String {
    let snapshot = accessible_names
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

fn wait_for_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if contains_accessible_name(&names, expected_name) {
                    return Ok(());
                }
                last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                    &names,
                    &[
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Status",
                        "Busy",
                        "Save",
                        "Reload",
                        "Connection / Port",
                        "Timeout",
                        "Warning",
                        "Interval",
                        "Media Search",
                        "view: media-search",
                    ],
                ));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_any_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_names: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if let Some(found) = expected_names
                    .iter()
                    .find(|expected| contains_accessible_name(&names, expected))
                {
                    return Ok((*found).to_owned());
                }
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            let expected_list = expected_names
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree; last accessibility read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_accessible_name_fragment<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_fragment: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if contains_accessible_name_fragment(&names, expected_fragment) {
                    return Ok(());
                }
                last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                    &names,
                    &[
                        expected_fragment,
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Status",
                        "Busy",
                        "Media Search",
                    ],
                ));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for accessibility name containing {expected_fragment:?}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for accessibility name containing {expected_fragment:?}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn invoke_named_control_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_snapshot = None;
    loop {
        match driver.invoke_named_control(window, name, control_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    let snapshot = driver
                        .accessible_names(window)
                        .map(|names| {
                            render_accessible_name_snapshot_for_patterns(
                                &names,
                                &[name, "Save", "Reset", "Reload", "Configuration", "view:"],
                            )
                        })
                        .unwrap_or_else(|_| "unavailable".to_owned());
                    return Err(format!(
                        "timed out invoking {} named {name:?}; last error: {error}; last snapshot: {}",
                        control_kind.label(),
                        if last_snapshot.is_some() {
                            last_snapshot.take().unwrap()
                        } else {
                            snapshot
                        }
                    ));
                }
                last_snapshot = driver.accessible_names(window).ok().map(|names| {
                    render_accessible_name_snapshot_for_patterns(
                        &names,
                        &[name, "Save", "Reset", "Reload", "Configuration", "view:"],
                    )
                });
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn invoke_menu_command_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    command_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = driver.invoke_named_control(window, menu_name, NativeControlKind::Any);
        thread::sleep(Duration::from_millis(100));
        match driver.invoke_named_control(window, command_name, command_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out invoking menu command {menu_name:?}->{command_name:?}; last error: {error}"
                    ));
                }
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn wait_for_named_control_count<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.count_named_controls(window, name, control_kind) {
            Ok(count) if count == expected_count => return Ok(()),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}; last count error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn invoke_menu_command_with_fallback<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        menu_name,
        command_name,
        NativeControlKind::MenuItem,
        timeout,
    ) {
        invoke_menu_command_with_wait(
            driver,
            window,
            menu_name,
            command_name,
            NativeControlKind::Any,
            timeout,
        )
        .map_err(|fallback_error| {
            format!(
                "failed to invoke {menu_name} -> {command_name} through menu item ({primary_error}); fallback also failed: {fallback_error}"
            )
        })
    } else {
        Ok(())
    }
}

fn navigate_to_view_with_fallback<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    button_name: &str,
    view_name: &str,
    fallback_menu_name: &str,
    fallback_command_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    if let Ok(accessible_names) = driver.accessible_names(window)
        && contains_accessible_name(&accessible_names, view_name)
    {
        return Ok(());
    }

    let sidebar_timeout = timeout.min(Duration::from_millis(800));
    let sidebar_result = invoke_named_control_with_wait(
        driver,
        window,
        button_name,
        NativeControlKind::Button,
        timeout,
    )
    .and_then(|_| wait_for_accessible_name(driver, window, view_name, sidebar_timeout));
    if sidebar_result.is_ok() {
        return Ok(());
    }

    let sidebar_error = sidebar_result.err().unwrap_or_else(|| {
        format!("sidebar navigation to {view_name:?} did not complete successfully")
    });
    invoke_menu_command_with_fallback(
        driver,
        window,
        fallback_menu_name,
        fallback_command_name,
        timeout,
    )
    .map_err(|menu_error| {
        format!(
            "failed to navigate to {view_name:?}; sidebar attempt failed: {sidebar_error}; menu fallback failed: {menu_error}"
        )
    })?;
    wait_for_accessible_name(driver, window, view_name, timeout).map_err(|wait_error| {
        format!(
            "menu fallback reached {fallback_menu_name} -> {fallback_command_name}, but {view_name:?} never appeared after sidebar failure ({sidebar_error}): {wait_error}"
        )
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", native_smoke_usage());
        return;
    }
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("syncplay-gui-native-smoke failed: {error}");
            std::process::exit(2);
        }
    };

    match run_native_smoke(&options) {
        Ok(report) => {
            print!("{}", report.render(options.format));
        }
        Err(error) => {
            print!("{}", render_error(&error, options.format));
            std::process::exit(1);
        }
    }
}
