use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use sorotte_client_app::app_boundary::{
    application::ClientApplication,
    commands::parse_seek_time_seconds_legacy_like,
    state::{StoredClientSettingsMvp, StreamingPlaybackConfig},
};
use sorotte_player_api::{PlayerAdapter, PlayerCommand, PlayerError};
use sorotte_player_mpv::MpvAdapter;
#[cfg(test)]
use sorotte_player_mpv::SimulatedPlayer;
use sorotte_secret::RedactedCommandArgs;

use crate::client_args::LegacyClientArgOverrides;
use crate::client_config::{ClientLoopConfig, create_client_session};
use crate::env_support::{
    env_flag_enabled, env_trimmed, env_u32, parse_env_bool_legacy_compatible,
    parse_env_non_negative_f64_legacy_compatible,
};
use crate::stored_settings::apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible;

mod attached_startup;
mod env_config;
mod explicit_args;
mod external_launch;
mod managed_process;
mod program_resolution;
mod types;

use self::env_config::explicit_mpv_ipc_path_from_env;
use self::explicit_args::emit_legacy_explicit_mpv_ipc_startup_player_arg_diagnostics_legacy_compatible;
use self::program_resolution::managed_mpv_launch_program_requires_existing_file_legacy_compatible;

pub(super) use self::attached_startup::apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible;
#[cfg(all(test, windows))]
pub(super) use self::attached_startup::retry_explicit_mpv_ipc_startup_player_command_legacy_compatible;
pub(super) use self::env_config::{
    apply_legacy_client_arg_managed_mpv_overrides, managed_mpv_launch_env_config_from_env,
};
pub(super) use self::explicit_args::analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible;
#[cfg(test)]
pub(super) use self::explicit_args::{
    legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible,
    parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible,
};
#[cfg(all(test, windows))]
pub(super) use self::external_launch::spawn_legacy_external_player_from_spec_legacy_compatible;
#[cfg(test)]
pub(super) use self::external_launch::{
    legacy_external_player_launch_spec_from_overrides_legacy_compatible,
    legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible,
    should_skip_legacy_external_player_launch_due_to_mpv_integration_env,
};
pub(super) use self::external_launch::{
    legacy_player_path_compatibility_warning_line_legacy_compatible,
    legacy_player_path_requests_managed_mpv_legacy_compatible,
    spawn_legacy_external_player_if_requested_legacy_compatible,
};
#[cfg(all(test, windows))]
pub(super) use self::managed_process::connect_mpv_adapter_with_retry;
#[cfg(test)]
pub(super) use self::managed_process::managed_mpv_launch_base_args_legacy_compatible;
pub(super) use self::managed_process::{
    ManagedMpvProcessGuard, create_client_runtime_with_managed_mpv_support,
};
pub(super) use self::program_resolution::{
    find_default_managed_mpv_bin, resolve_managed_mpv_launch_program_legacy_compatible,
};
pub(super) use self::types::LegacyExplicitMpvIpcStartupPlayerArgs;
pub(super) use self::types::{
    LegacyExplicitMpvIpcStartupPlayerArgAnalysis, LegacyExplicitMpvIpcStartupPlayerArgDiagnostics,
    LegacyExplicitMpvIpcStartupPlayerCommand, LegacyExternalPlayerLaunchSpec,
    ManagedMpvLaunchEnvConfig,
};
