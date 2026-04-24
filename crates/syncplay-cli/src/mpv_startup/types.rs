use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ManagedMpvLaunchEnvConfig {
    pub(crate) enabled: bool,
    pub(crate) mpv_bin: Option<PathBuf>,
    pub(crate) media_file: Option<PathBuf>,
    pub(crate) extra_args: Vec<String>,
    pub(crate) ipc_path: Option<String>,
    pub(crate) connect_timeout_ms: Option<u32>,
    pub(crate) connect_poll_interval_ms: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LegacyExplicitMpvIpcStartupPlayerArgs {
    pub(crate) paused: Option<bool>,
    pub(crate) start_position_seconds: Option<f64>,
    pub(crate) playback_rate: Option<f64>,
    pub(crate) muted: Option<bool>,
    pub(crate) volume: Option<f64>,
    pub(crate) deinterlace: Option<bool>,
    pub(crate) keepaspect: Option<bool>,
    pub(crate) keepaspect_window: Option<bool>,
    pub(crate) fullscreen: Option<bool>,
    pub(crate) ontop: Option<bool>,
    pub(crate) border: Option<bool>,
    pub(crate) force_window: Option<bool>,
    pub(crate) keep_open: Option<bool>,
    pub(crate) keep_open_pause: Option<bool>,
    pub(crate) cursor_autohide_fs_only: Option<bool>,
    pub(crate) stop_screensaver: Option<bool>,
    pub(crate) sub_visibility: Option<bool>,
    pub(crate) osd_bar: Option<bool>,
    pub(crate) window_maximized: Option<bool>,
    pub(crate) window_minimized: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LegacyExplicitMpvIpcStartupPlayerArgDiagnostics {
    pub(crate) supported_tokens: Vec<String>,
    pub(crate) malformed_tokens: Vec<String>,
    pub(crate) unsupported_tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyExplicitMpvIpcStartupPlayerCommand {
    SetOptionString { name: String, value: String },
    ApplyProfile { profile: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LegacyExplicitMpvIpcStartupPlayerArgAnalysis {
    pub(crate) parsed: LegacyExplicitMpvIpcStartupPlayerArgs,
    pub(crate) runtime_commands: Vec<LegacyExplicitMpvIpcStartupPlayerCommand>,
    pub(crate) diagnostics: LegacyExplicitMpvIpcStartupPlayerArgDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyExternalPlayerLaunchSpec {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
}
