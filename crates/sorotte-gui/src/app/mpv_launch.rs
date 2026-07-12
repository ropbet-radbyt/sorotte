use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sorotte_client_app::app_boundary::state::{
    ClientConfig, EffectiveMpvStreamingOption, StoredClientSettingsMvp,
};
use sorotte_player_api::PlayerAdapter;
use sorotte_player_mpv::{LegacySyncplayUiSettings, MpvAdapter};
use sorotte_secret::RedactedCommandArgs;

use super::child_process::configure_gui_child_process;

const DEFAULT_MANAGED_MPV_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_MANAGED_MPV_CONNECT_POLL_INTERVAL_MS: u64 = 50;
const LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER: &str = "-- sorotte-chat-input-bridge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManagedMpvSettingsDecision {
    NotConfigured,
    UnsupportedConfiguredPlayer { player_path: String },
    Launch(Box<ManagedMpvLaunchConfig>),
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ManagedMpvLaunchConfig {
    pub(crate) requested_player_path: String,
    pub(crate) program: PathBuf,
    pub(crate) streaming_args: Vec<String>,
    pub(crate) effective_streaming_options: Vec<EffectiveMpvStreamingOption>,
    pub(crate) extra_args: Vec<String>,
    pub(crate) ui_settings: LegacySyncplayUiSettings,
}

impl std::fmt::Debug for ManagedMpvLaunchConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedMpvLaunchConfig")
            .field("requested_player_path", &self.requested_player_path)
            .field("program", &self.program)
            .field(
                "streaming_args",
                &RedactedCommandArgs::from_args(&self.streaming_args),
            )
            .field(
                "effective_streaming_options",
                &self.effective_streaming_options,
            )
            .field(
                "extra_args",
                &RedactedCommandArgs::from_args(&self.extra_args),
            )
            .field("ui_settings", &self.ui_settings)
            .finish()
    }
}

impl ManagedMpvLaunchConfig {
    pub(crate) fn matches_process_target(&self, other: &Self) -> bool {
        self.requested_player_path == other.requested_player_path
            && self.program == other.program
            && self.streaming_args == other.streaming_args
            && self.extra_args == other.extra_args
    }
}

#[derive(Debug)]
pub(crate) struct ManagedMpvProcessGuard {
    child: Child,
    ipc_cleanup_path: Option<PathBuf>,
}

impl ManagedMpvProcessGuard {
    pub(crate) fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        self.child
            .try_wait()
            .map_err(|error| format!("failed checking GUI-owned mpv process state: {error}"))
    }
}

impl Drop for ManagedMpvProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(path) = self.ipc_cleanup_path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn managed_mpv_settings_decision_from_settings(
    settings: Option<&StoredClientSettingsMvp>,
) -> ManagedMpvSettingsDecision {
    let Some(settings) = settings else {
        return ManagedMpvSettingsDecision::NotConfigured;
    };
    let Some(player_path) = settings
        .player_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return ManagedMpvSettingsDecision::NotConfigured;
    };
    if !legacy_player_path_requests_managed_mpv_legacy_compatible(player_path) {
        return ManagedMpvSettingsDecision::UnsupportedConfiguredPlayer {
            player_path: player_path.to_owned(),
        };
    }

    let extra_args = settings
        .per_player_arguments
        .as_ref()
        .and_then(|arguments| arguments.get(player_path))
        .cloned()
        .unwrap_or_default();
    let streaming = ClientConfig::resolve(settings).config.playback.streaming;
    let streaming_args = streaming.mpv_arguments();
    let effective_streaming_options = streaming.effective_mpv_options(&extra_args);
    let program = resolve_managed_mpv_launch_program_legacy_compatible(Path::new(player_path));
    ManagedMpvSettingsDecision::Launch(Box::new(ManagedMpvLaunchConfig {
        requested_player_path: player_path.to_owned(),
        program,
        streaming_args,
        effective_streaming_options,
        extra_args,
        ui_settings: legacy_syncplay_ui_settings_from_stored_settings(Some(settings)),
    }))
}

pub(crate) fn apply_legacy_syncplay_ui_settings_to_mpv_adapter(
    player: &mut MpvAdapter,
    ui_settings: &LegacySyncplayUiSettings,
) -> Result<(), String> {
    player
        .configure_legacy_syncplay_ui_settings(ui_settings.clone())
        .map_err(|error| format!("failed to configure mpv OSD/chat settings: {error}"))?;
    player
        .set_option_string("drag-and-drop", "no")
        .map_err(|error| format!("failed to disable mpv drag-and-drop handling: {error}"))?;
    if let Err(error) = player.set_option_string("ytdl", "yes") {
        eprintln!(
            "warning: failed to enable mpv yt-dlp hook via GUI JSON IPC: {}",
            error
        );
    }
    if (ui_settings.chat_output_enabled || ui_settings.chat_input_enabled)
        && let Some(script_path) = find_legacy_syncplayintf_script_path()
        && let Err(error) = player.load_legacy_syncplayintf_script(&script_path)
    {
        eprintln!(
            "warning: failed to load legacy mpv syncplayintf.lua from '{}' via GUI JSON IPC: {}",
            script_path.display(),
            error
        );
    }

    Ok(())
}

pub(crate) fn apply_effective_streaming_options_to_mpv_adapter(
    player: &mut MpvAdapter,
    options: &[EffectiveMpvStreamingOption],
) -> Result<(), String> {
    for option in options {
        player
            .set_option_string(&option.name, &option.effective_value)
            .map_err(|error| {
                format!(
                    "failed to apply mpv streaming option {}={}: {error}",
                    option.name, option.effective_value
                )
            })?;
    }
    Ok(())
}

pub(crate) fn spawn_managed_mpv_and_attach(
    config: &ManagedMpvLaunchConfig,
    path_prefixes: &[PathBuf],
    downloader_path: Option<&Path>,
) -> Result<(MpvAdapter, ManagedMpvProcessGuard), String> {
    if managed_mpv_launch_program_requires_existing_file_legacy_compatible(&config.program)
        && !config.program.is_file()
    {
        return Err(format!(
            "managed mpv binary does not exist: {}",
            config.program.display()
        ));
    }

    let (ipc_path, ipc_cleanup_path) = generate_managed_mpv_ipc_path()?;
    let connect_timeout = Duration::from_millis(DEFAULT_MANAGED_MPV_CONNECT_TIMEOUT_MS);
    let connect_poll_interval = Duration::from_millis(DEFAULT_MANAGED_MPV_CONNECT_POLL_INTERVAL_MS);

    let mut command = Command::new(&config.program);
    if let Some(parent) = config.program.parent() {
        command.current_dir(parent);
    }
    if !path_prefixes.is_empty() {
        let mut joined_paths = path_prefixes.to_vec();
        if let Some(existing_path) = env::var_os("PATH") {
            joined_paths.extend(env::split_paths(&existing_path));
        }
        let joined = env::join_paths(joined_paths)
            .map_err(|error| format!("failed to construct PATH for managed mpv: {error}"))?;
        command.env("PATH", joined);
    }
    command.args(managed_mpv_launch_args(
        &ipc_path,
        &config.streaming_args,
        &config.extra_args,
        downloader_path,
    ));
    configure_gui_child_process(&mut command);

    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to start managed mpv '{}': {error}",
                config.program.display()
            )
        })?;
    let guard = ManagedMpvProcessGuard {
        child,
        ipc_cleanup_path,
    };
    let adapter = connect_mpv_adapter_with_retry(&ipc_path, connect_timeout, connect_poll_interval)
        .map_err(|error| {
            format!(
                "managed mpv launched but GUI JSON IPC attach failed (mpv_bin={}, ipc={}): {error}",
                config.program.display(),
                ipc_path
            )
        })?;
    Ok((adapter, guard))
}

fn managed_mpv_launch_args(
    ipc_path: &str,
    streaming_args: &[String],
    extra_args: &[String],
    downloader_path: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "--no-terminal".to_owned(),
        "--pause".to_owned(),
        "--force-window=yes".to_owned(),
        "--idle=yes".to_owned(),
        "--keep-open=always".to_owned(),
        "--keep-open-pause=yes".to_owned(),
        "--drag-and-drop=no".to_owned(),
        "--ytdl=yes".to_owned(),
    ];
    args.extend(streaming_args.iter().cloned());
    if let Some(path) = downloader_path {
        args.push(format!(
            "--script-opts-append=ytdl_hook-ytdl_path={}",
            path.display()
        ));
    }
    args.push(format!("--input-ipc-server={ipc_path}"));
    args.extend(extra_args.iter().cloned());
    args
}

pub(crate) fn legacy_syncplay_ui_settings_from_stored_settings(
    settings: Option<&StoredClientSettingsMvp>,
) -> LegacySyncplayUiSettings {
    let mut resolved = LegacySyncplayUiSettings::default();
    let Some(settings) = settings else {
        return resolved;
    };

    if let Some(show_osd) = settings.show_osd {
        resolved.show_osd = show_osd;
    }
    if let Some(chat_output_enabled) = settings.chat_output_enabled {
        resolved.chat_output_enabled = chat_output_enabled;
    }
    if let Some(chat_input_enabled) = settings.chat_input_enabled {
        resolved.chat_input_enabled = chat_input_enabled;
    }
    if let Some(chat_input_font_underline) = settings.chat_input_font_underline {
        resolved.chat_input_font_underline = chat_input_font_underline;
    }
    if let Some(chat_input_font_family) = settings
        .chat_input_font_family
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_font_family = chat_input_font_family.to_owned();
    }
    if let Some(chat_input_relative_font_size) = settings
        .chat_input_relative_font_size
        .filter(|value| *value > 0)
    {
        resolved.chat_input_relative_font_size = chat_input_relative_font_size;
    }
    if let Some(chat_input_font_weight) = settings.chat_input_font_weight {
        resolved.chat_input_font_weight = chat_input_font_weight;
    }
    if let Some(chat_input_font_color) = settings
        .chat_input_font_color
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_font_color = chat_input_font_color.to_owned();
    }
    if let Some(chat_input_position) = settings
        .chat_input_position
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_input_position = chat_input_position.to_owned();
    }
    if let Some(chat_direct_input) = settings.chat_direct_input {
        resolved.chat_direct_input = chat_direct_input;
    }
    if let Some(chat_output_font_underline) = settings.chat_output_font_underline {
        resolved.chat_output_font_underline = chat_output_font_underline;
    }
    if let Some(chat_output_font_family) = settings
        .chat_output_font_family
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_output_font_family = chat_output_font_family.to_owned();
    }
    if let Some(chat_output_relative_font_size) = settings
        .chat_output_relative_font_size
        .filter(|value| *value > 0)
    {
        resolved.chat_output_relative_font_size = chat_output_relative_font_size;
    }
    if let Some(chat_output_font_weight) = settings.chat_output_font_weight {
        resolved.chat_output_font_weight = chat_output_font_weight;
    }
    if let Some(chat_output_mode) = settings
        .chat_output_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resolved.chat_output_mode = chat_output_mode.to_owned();
    }
    if let Some(chat_max_lines) = settings.chat_max_lines.filter(|value| *value > 0) {
        resolved.chat_max_lines = chat_max_lines;
    }
    if let Some(chat_top_margin) = settings.chat_top_margin.filter(|value| *value >= 0) {
        resolved.chat_top_margin = chat_top_margin;
    }
    if let Some(chat_left_margin) = settings.chat_left_margin.filter(|value| *value >= 0) {
        resolved.chat_left_margin = chat_left_margin;
    }
    if let Some(chat_bottom_margin) = settings.chat_bottom_margin.filter(|value| *value >= 0) {
        resolved.chat_bottom_margin = chat_bottom_margin;
    }
    if let Some(chat_move_osd) = settings.chat_move_osd {
        resolved.chat_move_osd = chat_move_osd;
    }
    if let Some(chat_osd_margin) = settings.chat_osd_margin.filter(|value| *value >= 0) {
        resolved.chat_osd_margin = chat_osd_margin;
    }
    resolved.notification_timeout_ms = timeout_ms_from_stored_client_setting(
        settings.notification_timeout_seconds,
        resolved.notification_timeout_ms,
    );
    resolved.alert_timeout_ms = timeout_ms_from_stored_client_setting(
        settings.alert_timeout_seconds,
        resolved.alert_timeout_ms,
    );
    resolved.chat_timeout_ms = timeout_ms_from_stored_client_setting(
        settings.chat_timeout_seconds,
        resolved.chat_timeout_ms,
    );
    resolved
}

fn timeout_ms_from_stored_client_setting(value: Option<i64>, default_ms: u64) -> u64 {
    value
        .and_then(|seconds| {
            let seconds = u64::try_from(seconds).ok()?;
            seconds.checked_mul(1_000)
        })
        .unwrap_or(default_ms)
}

fn legacy_syncplayintf_script_candidate_paths() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest_dir.join("../../resources/syncplayintf.lua"),
        manifest_dir
            .join("../../.interop-cache/syncplay-legacy/syncplay/resources/syncplayintf.lua"),
        manifest_dir.join("../../../syncplay/syncplay/resources/syncplayintf.lua"),
    ]
}

fn legacy_syncplayintf_chat_input_bridge_source() -> &'static str {
    r#"
-- sorotte-chat-input-bridge
local syncplay_rust_original_handle_enter = handle_enter
function handle_enter()
    if repl_active and line ~= '' then
        local syncplay_rust_chat_line = line
        if opts['backslashSubstituteCharacter'] ~= nil then
            syncplay_rust_chat_line = string.gsub(syncplay_rust_chat_line, opts['backslashSubstituteCharacter'], "\\")
        end
        mp.commandv("script-message", "syncplayintf-chat", syncplay_rust_chat_line)
    end
    syncplay_rust_original_handle_enter()
end
"#
}

fn legacy_syncplayintf_script_source_with_chat_input_bridge(source: &str) -> String {
    if source.contains(LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER) {
        return source.to_owned();
    }

    let mut patched = source.to_owned();
    if !patched.ends_with('\n') {
        patched.push('\n');
    }
    patched.push_str(legacy_syncplayintf_chat_input_bridge_source());
    patched
}

fn prepare_legacy_syncplayintf_script_path(source_path: &Path) -> Result<PathBuf, String> {
    let source = fs::read_to_string(source_path).map_err(|error| {
        format!(
            "failed to read syncplayintf.lua from '{}': {error}",
            source_path.display()
        )
    })?;
    if source.contains(LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER) {
        return Ok(source_path.to_path_buf());
    }

    let patched = legacy_syncplayintf_script_source_with_chat_input_bridge(&source);
    let target_path =
        std::env::temp_dir().join(format!("sorotte-syncplayintf-{}.lua", std::process::id()));
    fs::write(&target_path, patched).map_err(|error| {
        format!(
            "failed to write patched syncplayintf.lua to '{}': {error}",
            target_path.display()
        )
    })?;
    Ok(target_path)
}

fn find_legacy_syncplayintf_script_path() -> Option<PathBuf> {
    let source_path = legacy_syncplayintf_script_candidate_paths()
        .into_iter()
        .find(|candidate| candidate.is_file())?;
    Some(prepare_legacy_syncplayintf_script_path(&source_path).unwrap_or(source_path))
}

#[cfg(windows)]
fn managed_mpv_launch_candidate_file_names_legacy_compatible() -> &'static [&'static str] {
    &["mpv.exe", "mpv.com"]
}

#[cfg(not(windows))]
fn managed_mpv_launch_candidate_file_names_legacy_compatible() -> &'static [&'static str] {
    &["mpv"]
}

fn push_unique_pathbuf_legacy_compatible(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

pub(crate) fn autodetect_mpv_player_path_legacy_compatible() -> Option<String> {
    autodetect_mpv_player_path_legacy_compatible_from_lookup(&|name| env::var(name).ok())
}

pub(crate) fn autodetect_mpv_player_path_legacy_compatible_from_lookup<F>(
    lookup: &F,
) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut candidates = Vec::new();

    if let Some(path_env) = lookup("PATH")
        && !path_env.trim().is_empty()
    {
        for directory in env::split_paths(&OsString::from(path_env)) {
            for file_name in managed_mpv_launch_candidate_file_names_legacy_compatible() {
                push_unique_pathbuf_legacy_compatible(&mut candidates, directory.join(file_name));
            }
        }
    }

    #[cfg(windows)]
    {
        for env_name in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            let Some(root) = lookup(env_name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            for candidate in [
                PathBuf::from(&root).join("mpv").join("mpv.exe"),
                PathBuf::from(&root)
                    .join("Programs")
                    .join("mpv")
                    .join("mpv.exe"),
            ] {
                push_unique_pathbuf_legacy_compatible(&mut candidates, candidate);
            }
        }
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

fn resolve_managed_mpv_launch_program_legacy_compatible(requested: &Path) -> PathBuf {
    let mut candidates = vec![requested.to_path_buf()];
    if requested.is_dir() || !requested.exists() {
        for file_name in managed_mpv_launch_candidate_file_names_legacy_compatible() {
            push_unique_pathbuf_legacy_compatible(&mut candidates, requested.join(file_name));
        }
    }
    if !requested.exists()
        && let Some(parent) = requested.parent()
        && let Some(file_name) = requested.file_name().and_then(|value| value.to_str())
    {
        let normalized = file_name.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "mpv" | "mpv.exe" | "mpv.com") {
            for candidate_file_name in managed_mpv_launch_candidate_file_names_legacy_compatible() {
                push_unique_pathbuf_legacy_compatible(
                    &mut candidates,
                    parent.join(candidate_file_name),
                );
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| requested.to_path_buf())
}

fn managed_mpv_launch_program_requires_existing_file_legacy_compatible(path: &Path) -> bool {
    path.is_absolute()
        || path
            .to_string_lossy()
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
}

fn legacy_player_path_requests_managed_mpv_legacy_compatible(player_path: &str) -> bool {
    let trimmed = player_path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.replace('\\', "/").to_ascii_lowercase();
    if normalized.contains("mpvnet") || !normalized.contains("mpv") {
        return false;
    }

    let file_name = Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str())
        .trim()
        .to_ascii_lowercase();
    if matches!(file_name.as_str(), "mpv" | "mpv.exe" | "mpv.com") {
        return true;
    }

    let requested = Path::new(trimmed);
    let resolved = resolve_managed_mpv_launch_program_legacy_compatible(requested);
    resolved.is_file()
        || !managed_mpv_launch_program_requires_existing_file_legacy_compatible(&resolved)
}

fn connect_mpv_adapter_with_retry(
    ipc_path: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<MpvAdapter, String> {
    let started = std::time::Instant::now();
    let mut last_error = None;
    while started.elapsed() < timeout {
        match MpvAdapter::with_json_ipc(ipc_path) {
            Ok(adapter) => return Ok(adapter),
            Err(error) => {
                last_error = Some(error.to_string());
                std::thread::sleep(poll_interval);
            }
        }
    }

    Err(format!(
        "timed out after {:?} waiting for mpv JSON IPC at '{}' (poll={:?}); last error: {}",
        timeout,
        ipc_path,
        poll_interval,
        last_error.as_deref().unwrap_or("<none>")
    ))
}

fn generate_managed_mpv_ipc_path() -> Result<(String, Option<PathBuf>), String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system time should be after unix epoch: {error}"))?
        .as_millis();
    #[cfg(windows)]
    {
        Ok((
            format!(r"\\.\pipe\sorotte-gui-mpv-{}-{unique}", std::process::id()),
            None,
        ))
    }
    #[cfg(not(windows))]
    {
        let path = std::env::temp_dir().join(format!(
            "sorotte-gui-mpv-{}-{unique}.sock",
            std::process::id()
        ));
        let path_str = path.to_string_lossy().into_owned();
        Ok((path_str, Some(path)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{
        LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER, ManagedMpvLaunchConfig,
        ManagedMpvSettingsDecision, autodetect_mpv_player_path_legacy_compatible_from_lookup,
        legacy_syncplayintf_script_source_with_chat_input_bridge, managed_mpv_launch_args,
        managed_mpv_settings_decision_from_settings,
    };
    use sorotte_client_app::app_boundary::state::{
        StoredClientSettingsMvp, StreamingPlaybackConfig,
    };
    use sorotte_player_mpv::LegacySyncplayUiSettings;

    #[test]
    fn managed_mpv_launch_config_debug_redacts_free_form_arguments() {
        let config = ManagedMpvLaunchConfig {
            requested_player_path: "mpv".to_owned(),
            program: PathBuf::from("mpv"),
            streaming_args: StreamingPlaybackConfig::default().mpv_arguments(),
            effective_streaming_options: Vec::new(),
            extra_args: vec![
                "--http-header-fields=Authorization: Bearer GUI_PLAYER_ARG_CANARY".to_owned(),
                "--cookies-file=C:/private/GUI_PLAYER_ARG_CANARY.txt".to_owned(),
                "https://media.example/video?Signature=GUI_PLAYER_ARG_CANARY".to_owned(),
            ],
            ui_settings: LegacySyncplayUiSettings::default(),
        };

        let rendered = format!("{config:?}");

        assert!(rendered.contains("RedactedCommandArgs"));
        assert!(!rendered.contains("GUI_PLAYER_ARG_CANARY"));
        assert!(!rendered.contains("Authorization: Bearer"));
        assert!(!rendered.contains("--cookies-file"));
        assert!(!rendered.contains("?Signature="));
    }

    #[test]
    fn managed_mpv_settings_decision_uses_saved_player_path_and_per_player_arguments() {
        let mut per_player_arguments = BTreeMap::new();
        per_player_arguments.insert(
            "C:/Program Files/mpv/mpv.exe".to_owned(),
            vec![
                "--profile=syncplay".to_owned(),
                "--keep-open=yes".to_owned(),
            ],
        );
        let decision =
            managed_mpv_settings_decision_from_settings(Some(&StoredClientSettingsMvp {
                player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
                per_player_arguments: Some(per_player_arguments),
                show_osd: Some(false),
                chat_input_enabled: Some(true),
                ..StoredClientSettingsMvp::default()
            }));

        let ManagedMpvSettingsDecision::Launch(config) = decision else {
            panic!("expected managed mpv launch config");
        };
        assert_eq!(config.requested_player_path, "C:/Program Files/mpv/mpv.exe");
        assert!(
            config
                .streaming_args
                .contains(&"--cache-pause-wait=5".to_owned())
        );
        assert_eq!(
            config.extra_args,
            vec![
                "--profile=syncplay".to_owned(),
                "--keep-open=yes".to_owned()
            ]
        );
        assert!(!config.ui_settings.show_osd);
        assert!(config.ui_settings.chat_input_enabled);
    }

    #[test]
    fn managed_mpv_settings_decision_rejects_non_mpv_saved_player_paths() {
        let decision =
            managed_mpv_settings_decision_from_settings(Some(&StoredClientSettingsMvp {
                player_path: Some("C:/Windows/System32/notepad.exe".to_owned()),
                ..StoredClientSettingsMvp::default()
            }));

        assert_eq!(
            decision,
            ManagedMpvSettingsDecision::UnsupportedConfiguredPlayer {
                player_path: "C:/Windows/System32/notepad.exe".to_owned()
            }
        );
    }

    #[test]
    fn managed_mpv_settings_decision_ignores_empty_player_paths() {
        assert_eq!(
            managed_mpv_settings_decision_from_settings(Some(&StoredClientSettingsMvp {
                player_path: Some("   ".to_owned()),
                ..StoredClientSettingsMvp::default()
            })),
            ManagedMpvSettingsDecision::NotConfigured
        );
    }

    #[test]
    fn legacy_syncplayintf_script_bridge_patch_is_idempotent() {
        let source = "function handle_enter()\nend\n";
        let once = legacy_syncplayintf_script_source_with_chat_input_bridge(source);
        let twice = legacy_syncplayintf_script_source_with_chat_input_bridge(&once);

        assert!(once.contains(LEGACY_SYNCPLAYINTF_CHAT_INPUT_BRIDGE_MARKER));
        assert_eq!(once, twice);
    }

    #[test]
    fn managed_mpv_launch_args_open_a_visible_idle_window_before_extra_args() {
        let args = managed_mpv_launch_args(
            r"\\.\pipe\sorotte-gui-mpv-test",
            &StreamingPlaybackConfig::default().mpv_arguments(),
            &["--profile=syncplay".to_owned()],
            None,
        );

        assert_eq!(
            args,
            vec![
                "--no-terminal".to_owned(),
                "--pause".to_owned(),
                "--force-window=yes".to_owned(),
                "--idle=yes".to_owned(),
                "--keep-open=always".to_owned(),
                "--keep-open-pause=yes".to_owned(),
                "--drag-and-drop=no".to_owned(),
                "--ytdl=yes".to_owned(),
                "--cache=auto".to_owned(),
                "--cache-pause=yes".to_owned(),
                "--cache-pause-initial=yes".to_owned(),
                "--cache-pause-wait=5".to_owned(),
                "--cache-secs=30".to_owned(),
                "--demuxer-max-bytes=150MiB".to_owned(),
                "--cache-on-disk=no".to_owned(),
                r"--input-ipc-server=\\.\pipe\sorotte-gui-mpv-test".to_owned(),
                "--profile=syncplay".to_owned(),
            ]
        );
    }

    #[test]
    fn managed_mpv_launch_args_include_explicit_ytdl_path_before_extra_args() {
        let args = managed_mpv_launch_args(
            r"\\.\pipe\sorotte-gui-mpv-test",
            &StreamingPlaybackConfig::default().mpv_arguments(),
            &["--profile=syncplay".to_owned()],
            Some(std::path::Path::new("C:/Tools/yt-dlp.exe")),
        );

        assert_eq!(
            args,
            vec![
                "--no-terminal".to_owned(),
                "--pause".to_owned(),
                "--force-window=yes".to_owned(),
                "--idle=yes".to_owned(),
                "--keep-open=always".to_owned(),
                "--keep-open-pause=yes".to_owned(),
                "--drag-and-drop=no".to_owned(),
                "--ytdl=yes".to_owned(),
                "--cache=auto".to_owned(),
                "--cache-pause=yes".to_owned(),
                "--cache-pause-initial=yes".to_owned(),
                "--cache-pause-wait=5".to_owned(),
                "--cache-secs=30".to_owned(),
                "--demuxer-max-bytes=150MiB".to_owned(),
                "--cache-on-disk=no".to_owned(),
                "--script-opts-append=ytdl_hook-ytdl_path=C:/Tools/yt-dlp.exe".to_owned(),
                r"--input-ipc-server=\\.\pipe\sorotte-gui-mpv-test".to_owned(),
                "--profile=syncplay".to_owned(),
            ]
        );
    }

    #[test]
    fn autodetect_mpv_player_path_prefers_path_entries() {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("syncplay-mpv-autodetect-{unique_suffix}"));
        std::fs::create_dir_all(&root).expect("autodetect temp root should exist");
        let mpv_path = root.join(if cfg!(windows) { "mpv.exe" } else { "mpv" });
        std::fs::write(&mpv_path, b"").expect("fake mpv should be written");

        let path_value = std::env::join_paths([root.clone()])
            .expect("path should join")
            .to_string_lossy()
            .into_owned();
        let detected = autodetect_mpv_player_path_legacy_compatible_from_lookup(&|name| {
            (name == "PATH").then_some(path_value.clone())
        });

        assert_eq!(
            detected.as_deref(),
            Some(mpv_path.to_string_lossy().as_ref())
        );

        let _ = std::fs::remove_file(&mpv_path);
        let _ = std::fs::remove_dir_all(&root);
    }
}
