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
const CONFIG_HOST_EDIT_INDEX: usize = 0;
const CONFIG_PORT_VALUE: &str = "8999";
const CONFIG_PORT_EDIT_INDEX: usize = 1;
const CONFIG_USERNAME_VALUE: &str = "smoke-user";
const CONFIG_USERNAME_EDIT_INDEX: usize = 2;
const CONFIG_ROOM_VALUE: &str = "smoke-room";
const CONFIG_ROOM_EDIT_INDEX: usize = 3;
const CONFIG_PLAYER_PATH_VALUE: &str = "C:\\Windows\\System32\\notepad.exe";
const CONFIG_PLAYER_PATH_EDIT_INDEX: usize = 5;
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
use native_smoke_runner::run_native_smoke;
use platform_driver::{NativeControlKind, NativeGuiDriver, PlatformNativeGuiDriver};

#[path = "sorotte-gui-native-smoke/native_smoke_accessibility.rs"]
mod native_smoke_accessibility;
#[path = "sorotte-gui-native-smoke/native_smoke_cli.rs"]
mod native_smoke_cli;
#[path = "sorotte-gui-native-smoke/native_smoke_setup.rs"]
mod native_smoke_setup;

use native_smoke_accessibility::*;
use native_smoke_cli::*;
use native_smoke_setup::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
