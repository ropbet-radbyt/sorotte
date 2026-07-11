use super::*;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ManagedMpvLaunchEnvConfig {
    pub(crate) enabled: bool,
    pub(crate) mpv_bin: Option<PathBuf>,
    pub(crate) media_file: Option<PathBuf>,
    pub(crate) extra_args: Vec<String>,
    pub(crate) ipc_path: Option<String>,
    pub(crate) connect_timeout_ms: Option<u32>,
    pub(crate) connect_poll_interval_ms: Option<u32>,
}

impl std::fmt::Debug for ManagedMpvLaunchEnvConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedMpvLaunchEnvConfig")
            .field("enabled", &self.enabled)
            .field("mpv_bin", &self.mpv_bin)
            .field("media_file_present", &self.media_file.is_some())
            .field(
                "extra_args",
                &RedactedCommandArgs::from_args(&self.extra_args),
            )
            .field("ipc_path", &self.ipc_path)
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("connect_poll_interval_ms", &self.connect_poll_interval_ms)
            .finish()
    }
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

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct LegacyExplicitMpvIpcStartupPlayerArgDiagnostics {
    pub(crate) supported_tokens: Vec<String>,
    pub(crate) malformed_tokens: Vec<String>,
    pub(crate) unsupported_tokens: Vec<String>,
}

impl std::fmt::Debug for LegacyExplicitMpvIpcStartupPlayerArgDiagnostics {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyExplicitMpvIpcStartupPlayerArgDiagnostics")
            .field(
                "supported_tokens",
                &RedactedCommandArgs::from_args(&self.supported_tokens),
            )
            .field(
                "malformed_tokens",
                &RedactedCommandArgs::from_args(&self.malformed_tokens),
            )
            .field(
                "unsupported_tokens",
                &RedactedCommandArgs::from_args(&self.unsupported_tokens),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum LegacyExplicitMpvIpcStartupPlayerCommand {
    SetOptionString { name: String, value: String },
    ApplyProfile { profile: String },
}

impl std::fmt::Debug for LegacyExplicitMpvIpcStartupPlayerCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetOptionString { name, .. } => formatter
                .debug_struct("SetOptionString")
                .field(
                    "option",
                    &RedactedCommandArgs::from_option_names(std::iter::once(name)),
                )
                .finish(),
            Self::ApplyProfile { .. } => formatter
                .debug_struct("ApplyProfile")
                .field("argument", &RedactedCommandArgs::from_count(1))
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LegacyExplicitMpvIpcStartupPlayerArgAnalysis {
    pub(crate) parsed: LegacyExplicitMpvIpcStartupPlayerArgs,
    pub(crate) runtime_commands: Vec<LegacyExplicitMpvIpcStartupPlayerCommand>,
    pub(crate) diagnostics: LegacyExplicitMpvIpcStartupPlayerArgDiagnostics,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LegacyExternalPlayerLaunchSpec {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
}

impl std::fmt::Debug for LegacyExternalPlayerLaunchSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyExternalPlayerLaunchSpec")
            .field("program", &self.program)
            .field("args", &RedactedCommandArgs::from_args(&self.args))
            .finish()
    }
}
