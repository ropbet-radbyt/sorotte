mod controlled_rooms;
mod display;
mod parser;
mod planner;
mod playlist;
mod types;

pub use controlled_rooms::{
    controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
};
pub use display::{
    local_input_error_output_line_legacy_compatible,
    localized_current_offset_message_legacy_compatible,
    localized_local_input_error_message_legacy_compatible,
    playlist_listing_message_legacy_compatible,
    playlist_listing_message_localized_legacy_compatible,
    render_local_input_display_lines_legacy_compatible,
};
pub use parser::{
    parse_local_input_chat_message, parse_local_input_command, parse_seek_time_seconds_legacy_like,
};
pub use planner::{
    plan_local_input_command_legacy_compatible, plan_local_input_dispatch_legacy_compatible,
    plan_local_offset_runtime_dispatch_legacy_compatible,
    plan_local_playlist_delete_runtime_dispatch_legacy_compatible,
    plan_local_playlist_select_runtime_dispatch_legacy_compatible,
    plan_local_runtime_dispatch_legacy_compatible,
    resolved_local_user_offset_seconds_legacy_compatible,
};
pub use playlist::playlist_index_in_bounds_legacy_compatible;
pub use types::{
    LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
    LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
    PlannedLocalRuntimeAction, PlannedLocalRuntimeDispatch,
};

#[cfg(test)]
pub(crate) use display::{
    local_command_help_footer_lines_legacy_compatible, local_command_help_lines_legacy_compatible,
    localized_unknown_command_message_legacy_compatible,
};

#[cfg(test)]
mod tests;
