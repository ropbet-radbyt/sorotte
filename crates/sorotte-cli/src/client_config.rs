use serde_json::{Map, Value};
use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, normalize_controlled_room_input_legacy_compatible,
    parse_autoplay_min_users_override_legacy_compatible,
    parse_unpause_action_mode_legacy_compatible,
};
use sorotte_client_core::{
    ClientSession, PrivacyMode, ReadinessAutoplayConfig, ReconnectStateRestoreCorrectionPolicyMode,
    UnpauseActionMode,
};
use sorotte_player_mpv::MpvAdapter;

use crate::env_support::{
    env_flag_enabled, env_flag_override, env_non_negative_f64, env_port, env_privacy_mode,
    env_string_list, env_trimmed, env_u32, env_usize,
};
#[cfg(test)]
use crate::{
    apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible, create_mpv_adapter_from_env,
};
mod env_build;
mod overrides;
mod runtime_inputs;
mod session_factory;
mod types;

pub(super) use self::env_build::build_client_loop_config_from_env;
#[cfg(test)]
pub(super) use self::env_build::normalize_controlled_room_input;
#[cfg(test)]
pub(super) use self::overrides::parse_reconnect_state_restore_correction_policy_mode_legacy_compatible;
pub(super) use self::overrides::{
    apply_chat_policy_overrides, apply_client_behavior_overrides,
    apply_readiness_autoplay_overrides,
};
pub(super) use self::runtime_inputs::{
    client_hello_features_legacy_compatible, derive_runtime_loop_inputs,
    shared_playlists_enabled_cli_legacy_compatible,
};
#[cfg(test)]
pub(super) use self::session_factory::create_client_runtime;
pub(super) use self::session_factory::create_client_session;
pub(super) use self::types::{
    ChatPolicyOverrides, ClientBehaviorOverrides, ClientLoopConfig, ReadinessAutoplayOverrides,
    RuntimeLoopInputs,
};
