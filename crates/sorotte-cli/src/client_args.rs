#[cfg(test)]
use sorotte_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg as shared_parse_host_and_optional_port_from_host_arg;
use sorotte_client_app::app_boundary::{
    compatibility::{
        syncplay_configuration_getter_ini_compat_entries,
        syncplay_configuration_getter_startup_compat_entries,
    },
    language::runtime_language_selection_line,
    state::{StoredClientSettings, normalize_controlled_room_input},
};
use sorotte_secret::SecretValue;

use crate::client_config::ClientLoopConfig;
use crate::mpv_startup::player_path_compatibility_warning_line;
mod apply;
mod force_gui;
mod help;
mod localization;
mod parser;
mod types;

pub(super) use self::apply::{
    apply_syncplay_client_arg_overrides, emit_syncplay_client_arg_compatibility_warnings,
    syncplay_unrecognized_arguments_diagnostic_line, validate_composed_client_endpoint,
};
pub(super) use self::force_gui::{
    should_halt_for_stored_force_gui_prompt, stored_force_gui_prompt_compatibility_line,
    syncplay_force_gui_prompt_compatibility_line,
};
pub(super) use self::help::print_syncplay_client_help;
#[cfg(test)]
pub(super) use self::localization::{
    localized_compatibility_input_label, localized_compatibility_note_label,
    localized_startup_compatibility_heading, localized_syncplay_ini_compatibility_heading,
};
#[cfg(not(test))]
pub(super) use self::parser::parse_syncplay_client_arg_overrides;
#[cfg(test)]
pub(super) use self::parser::{
    parse_host_and_optional_port_from_host_arg, parse_syncplay_client_arg_overrides,
};
pub(super) use self::types::{
    HostArgumentError, SyncplayClientArgOverrides, SyncplayClientArgumentIssue,
};
