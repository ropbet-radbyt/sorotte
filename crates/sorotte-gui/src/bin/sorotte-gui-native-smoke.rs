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

use sorotte_client_app::app_boundary::{
    persistence::{
        format_serialized_public_servers_list_legacy_compatible,
        load_sorotte_ini_stored_client_settings_mvp_from_path,
        upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    },
    state::{AutoplayThresholdOverride, StoredClientSettingsMvp},
};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};
use sorotte_compat::LegacyServerPythonPeerHarness;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct NativeSmokeOptions {
    binary_path: Option<PathBuf>,
    timeout: Duration,
    format: OutputFormat,
    input_mode: NativeInputMode,
    keep_open: bool,
    scenario_filters: Vec<String>,
}

struct NativeSmokeReport {
    input_mode: NativeInputMode,
    binary_path: String,
    pid: u32,
    window_title: String,
    menu_source: String,
    menu_labels: Vec<String>,
    menu_automation_ids: Vec<String>,
    menu_contract: String,
    accessible_name_count: usize,
    accessibility_contract: String,
    interaction_steps: Vec<String>,
    interaction_contract: String,
    capability_outcomes: Vec<NativeCapabilityOutcome>,
    duration_ms: u128,
    closed: bool,
}

#[derive(serde::Serialize)]
struct NativeCapabilityOutcome {
    capability_id: String,
    outcome: String,
    source: String,
    evidence: Vec<String>,
}

#[derive(Clone, Copy)]
struct TcpSessionBootstrap<'a> {
    host: &'a str,
    port: u16,
    username: &'a str,
    room: &'a str,
}

#[derive(Clone, Copy)]
enum NativeTcpBootstrap<'a> {
    Environment(TcpSessionBootstrap<'a>),
    SavedConfig,
}

#[derive(Clone, Copy)]
enum NativeNetworkMode<'a> {
    Detached,
    InProcessLoopback { username: &'a str, room: &'a str },
    TcpLoopback { bootstrap: NativeTcpBootstrap<'a> },
}

#[derive(Clone, Copy)]
struct GuiLaunchConfig<'a> {
    config_path: &'a Path,
    media_search_browse_path: &'a Path,
    open_media_file_path: &'a Path,
    public_servers_spec: &'a str,
    network_mode: NativeNetworkMode<'a>,
    attach_test_player: bool,
    drop_file_paths_spec: Option<&'a str>,
    drop_target: Option<&'a str>,
}

#[derive(Clone, Copy, Default)]
struct GuiLaunchTestOverrides<'a> {
    theme: Option<&'a str>,
    appdata_root: Option<&'a Path>,
    explicit_config_path_with_appdata_root: bool,
    config_storage_browse_path: Option<&'a Path>,
    test_player_observation_path: Option<&'a Path>,
    lifecycle_observation_path: Option<&'a Path>,
    shared_lifecycle_evidence_path: Option<&'a Path>,
    shared_lifecycle_run_id: Option<&'a str>,
    shared_lifecycle_emitter: Option<&'a str>,
    disable_startup_saved_connect: bool,
    player_settings_degraded: bool,
}

type PlaylistExchangeEvidence = (String, String, String, String, String);

struct MockSessionServer {
    address: String,
    port: u16,
    peer_rx: mpsc::Receiver<String>,
    hello_rx: mpsc::Receiver<String>,
    playlist_exchange_rx: Option<mpsc::Receiver<PlaylistExchangeEvidence>>,
    playstate_exchange_rx: Option<mpsc::Receiver<(String, String)>>,
    authoritative_tx: Option<mpsc::Sender<String>>,
    release_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

#[path = "sorotte-gui-native-smoke/native_smoke_runner.rs"]
mod native_smoke_runner;
const DEFAULT_PUBLIC_SERVERS_SPEC: &str =
    "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]";
const DEFAULT_UPDATE_CHECK_RESPONSE: &str =
    r#"{"version-status":"uptodate","version-message":"Sorotte is up to date."}"#;
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
const MAIN_WINDOW_ROOM_BROWSER_NAME: &str = "Room Browser";
const ROOM_SURFACE_AUTOMATION_ID: &str = "main-window-root";
const SETUP_SURFACE_AUTOMATION_ID: &str = "configuration-root";
const CONFIG_CONNECTION_TAB_AUTOMATION_ID: &str = "configuration:tab:connection";
const CONFIG_PLAYBACK_SEARCH_TAB_AUTOMATION_ID: &str = "configuration:tab:playback-search";
const CONFIG_PRIVACY_CHAT_TAB_AUTOMATION_ID: &str = "configuration:tab:privacy-chat";
const CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID: &str = "configuration:tab:interface-system";
const CONFIG_SAVE_AUTOMATION_ID: &str = "config-command:save";
const CONFIG_RELOAD_AUTOMATION_ID: &str = "config-command:reload";
const CONFIG_CLEAR_GUI_DATA_AUTOMATION_ID: &str = "config-command:clear-gui-data";
const CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID: &str = "config-command:confirm-clear-gui-data";
const CONFIG_CONNECT_ONCE_AUTOMATION_ID: &str = "config-command:connect-once";
const MODAL_TLS_TRUST_AUTOMATION_ID: &str = "shell:modal:tls:trust";
const MODAL_CLOSE_AUTOMATION_ID: &str = "shell:modal:close";
const MODAL_PLAYER_SETUP_RETRY_AUTOMATION_ID: &str = "shell:modal:player-setup:retry";
const MODAL_PLAYER_SETUP_OPEN_SETTINGS_AUTOMATION_ID: &str =
    "shell:modal:player-setup:open-settings";
const PUBLIC_SERVER_EDIT_LABEL_AUTOMATION_ID: &str = "public-servers:edit:label";
const PUBLIC_SERVER_EDIT_ADDRESS_AUTOMATION_ID: &str = "public-servers:edit:address";
const PUBLIC_SERVER_EDIT_COMMIT_AUTOMATION_ID: &str = "public-servers:edit:commit";
const PUBLIC_SERVER_CONNECT_AUTOMATION_ID: &str = "public-servers:command:connect";
#[cfg(target_os = "windows")]
const MAIN_WINDOW_CONTROLS_CONTAINER_NAME: &str = "Controls";
#[cfg(target_os = "windows")]
const MAIN_WINDOW_LOCAL_READY_BUTTON_NAME: &str = "Set Ready";
#[cfg(target_os = "windows")]
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
const CONFIG_HOST_AUTOMATION_ID: &str = "settings.connection.host";
const CONFIG_PORT_VALUE: &str = "8999";
const CONFIG_PORT_AUTOMATION_ID: &str = "settings.connection.port";
const CONFIG_USERNAME_VALUE: &str = "smoke-user";
const CONFIG_USERNAME_AUTOMATION_ID: &str = "settings.connection.username";
const CONFIG_ROOM_VALUE: &str = "smoke-room";
const CONFIG_ROOM_AUTOMATION_ID: &str = "settings.connection.room";
const CONFIG_PLAYER_PATH_VALUE: &str = "C:\\Windows\\System32\\notepad.exe";
const CONFIG_PLAYER_PATH_AUTOMATION_ID: &str = "settings.player.executable";
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
#[path = "sorotte-gui-native-smoke/platform_driver.rs"]
mod platform_driver;
use native_smoke_runner::{
    run_native_smoke, run_participant_status_system_from_args, run_real_mpv_vertical_from_args,
};
use platform_driver::{
    NativeAccessibilityNode, NativeControlKind, NativeGuiDriver, NativeInputMode,
    PlatformNativeGuiDriver,
};

#[path = "sorotte-gui-native-smoke/native_smoke_accessibility.rs"]
mod native_smoke_accessibility;
#[path = "sorotte-gui-native-smoke/native_smoke_cli.rs"]
mod native_smoke_cli;
#[path = "sorotte-gui-native-smoke/native_smoke_setup.rs"]
mod native_smoke_setup;
#[path = "sorotte-gui-native-smoke/visual_artifacts.rs"]
mod visual_artifacts;

use native_smoke_accessibility::*;
use native_smoke_cli::*;
use native_smoke_setup::*;

#[cfg(target_os = "windows")]
fn enable_native_smoke_dpi_awareness() {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware;

    // SAFETY: This is called before the smoke runner initializes COM, discovers windows, or
    // performs any coordinate-sensitive UI Automation/GDI work.
    unsafe {
        SetProcessDPIAware();
    }
}

fn main() {
    #[cfg(target_os = "windows")]
    enable_native_smoke_dpi_awareness();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--participant-status-system") {
        match run_participant_status_system_from_args(&args) {
            Ok(report) => {
                println!("{report}");
                return;
            }
            Err(error) => {
                eprintln!("sorotte-gui participant-status system proof failed: {error}");
                println!("{}", render_error(&error, OutputFormat::Json));
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|arg| arg == "--real-mpv-vertical") {
        match run_real_mpv_vertical_from_args(&args) {
            Ok(report) => {
                println!("{report}");
                return;
            }
            Err(error) => {
                eprintln!("sorotte-gui real-mpv vertical failed: {error}");
                println!("{}", render_error(&error, OutputFormat::Json));
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|arg| arg == "--visual-suite") {
        match visual_artifacts::run_visual_suite_from_args(&args) {
            Ok(report) => {
                println!("{report}");
                return;
            }
            Err(error) => {
                eprintln!("sorotte-gui visual suite failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", native_smoke_usage());
        return;
    }
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("sorotte-gui-native-smoke failed: {error}");
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
