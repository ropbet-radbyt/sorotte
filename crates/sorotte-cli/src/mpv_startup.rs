use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use sorotte_client_app::app_boundary::{
    application::ClientApplication,
    commands::parse_seek_time_seconds,
    state::{ClientConfig, StoredClientSettings},
};
use sorotte_player_api::{PlayerAdapter, PlayerCommand, PlayerError};
use sorotte_player_mpv::{MpvAdapter, SorotteBridgeHealth};
use sorotte_secret::RedactedCommandArgs;

use crate::client_args::SyncplayClientArgOverrides;
use crate::client_config::{ClientLoopConfig, create_client_session};
use crate::env_support::{
    env_flag_enabled, env_trimmed, env_u32, parse_env_bool, parse_env_non_negative_f64,
};
use crate::stored_settings::apply_syncplay_ui_settings_to_mpv_adapter;

mod attached_startup;
mod env_config;
mod explicit_args;
mod external_launch;
mod managed_process;
mod program_resolution;
mod types;

use self::env_config::explicit_mpv_ipc_path_from_env;
use self::explicit_args::emit_explicit_mpv_ipc_startup_player_arg_diagnostics;
use self::program_resolution::managed_mpv_launch_program_requires_existing_file;

pub(super) use self::attached_startup::apply_startup_file_to_attached_player_if_explicit_mpv_ipc;
#[cfg(all(test, windows))]
pub(super) use self::attached_startup::retry_explicit_mpv_ipc_startup_player_command;
pub(super) use self::env_config::{
    apply_syncplay_client_arg_managed_mpv_overrides, managed_mpv_launch_env_config_from_env,
};
pub(super) use self::explicit_args::analyze_explicit_mpv_ipc_startup_player_args;
#[cfg(test)]
pub(super) use self::explicit_args::{
    explicit_mpv_ipc_startup_player_arg_diagnostic_lines,
    parse_explicit_mpv_ipc_startup_player_args,
};
#[cfg(test)]
pub(super) use self::external_launch::spawn_external_player_from_spec;
#[cfg(test)]
pub(super) use self::external_launch::{
    external_player_launch_spec_from_overrides,
    non_mpv_player_path_ignored_by_mpv_integration_warning_line,
    should_skip_external_player_launch_due_to_mpv_integration_env,
};
pub(super) use self::external_launch::{
    player_path_compatibility_warning_line, player_path_requests_managed_mpv,
    spawn_external_player_if_requested,
};
#[cfg(all(test, windows))]
pub(super) use self::managed_process::connect_mpv_adapter_with_retry;
pub(super) use self::managed_process::{
    ManagedMpvProcessGuard, create_client_runtime_with_managed_mpv_support,
};
#[cfg(test)]
pub(super) use self::managed_process::{
    create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test,
    create_client_runtime_with_prepared_mpv_and_startup_health_for_test,
    create_client_runtime_with_prepared_mpv_for_test, managed_mpv_launch_base_args,
};
pub(super) use self::program_resolution::{
    find_default_managed_mpv_bin, resolve_managed_mpv_launch_program,
};
pub(super) use self::types::ExplicitMpvIpcStartupPlayerArgs;
pub(super) use self::types::{
    ExplicitMpvIpcStartupPlayerArgAnalysis, ExplicitMpvIpcStartupPlayerArgDiagnostics,
    ExplicitMpvIpcStartupPlayerCommand, ExternalPlayerLaunchSpec, ManagedMpvLaunchEnvConfig,
};
