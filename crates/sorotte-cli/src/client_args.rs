use sorotte_client_app::app_boundary::{
    compatibility::{
        legacy_configuration_getter_ini_compat_entries,
        legacy_configuration_getter_startup_compat_entries,
    },
    language::legacy_runtime_language_selection_line_legacy_compatible,
    state::{
        StoredClientSettingsMvp, normalize_controlled_room_input_legacy_compatible,
        parse_host_and_optional_port_from_host_arg_legacy_compatible as shared_parse_host_and_optional_port_from_host_arg_legacy_compatible,
    },
};

use crate::client_config::ClientLoopConfig;
use crate::mpv_startup::legacy_player_path_compatibility_warning_line_legacy_compatible;
mod apply;
mod force_gui;
mod help;
mod localization;
mod parser;
mod types;

pub(super) use self::apply::{
    apply_legacy_client_arg_overrides, emit_legacy_client_arg_compatibility_warnings,
};
pub(super) use self::force_gui::{
    legacy_force_gui_prompt_compatibility_line_legacy_compatible,
    should_halt_for_stored_force_gui_prompt_legacy_compatible,
    stored_force_gui_prompt_compatibility_line_legacy_compatible,
};
pub(super) use self::help::print_legacy_client_help;
#[cfg(test)]
pub(super) use self::localization::{
    localized_compatibility_input_label_legacy_compatible,
    localized_compatibility_note_label_legacy_compatible,
    localized_legacy_ini_compatibility_heading_legacy_compatible,
    localized_legacy_startup_compatibility_heading_legacy_compatible,
};
#[cfg(not(test))]
pub(super) use self::parser::parse_legacy_client_arg_overrides;
#[cfg(test)]
pub(super) use self::parser::{
    parse_host_and_optional_port_from_host_arg_legacy_compatible, parse_legacy_client_arg_overrides,
};
pub(super) use self::types::LegacyClientArgOverrides;
