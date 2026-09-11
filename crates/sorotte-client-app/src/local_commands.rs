mod controlled_rooms;
mod display;
mod parser;
mod planner;
mod playlist;
mod types;

pub use controlled_rooms::{controlled_room_base_name, generate_room_password};
pub use display::{
    local_input_error_output_line, localized_current_offset_message,
    localized_local_input_error_message, playlist_listing_message,
    playlist_listing_message_localized, render_local_input_display_lines,
};
pub use parser::{
    parse_local_input_chat_message, parse_local_input_command, parse_seek_time_seconds,
};
pub use planner::{
    plan_local_input_command, plan_local_input_dispatch, plan_local_offset_runtime_dispatch,
    plan_local_playlist_delete_runtime_dispatch, plan_local_playlist_select_runtime_dispatch,
    plan_local_runtime_dispatch, resolved_local_user_offset_seconds,
};
pub use playlist::playlist_index_in_bounds;
pub use types::{
    LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
    LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
    PlannedLocalRuntimeAction, PlannedLocalRuntimeDispatch,
};

#[cfg(test)]
pub(crate) use display::{
    local_command_help_footer_lines, local_command_help_lines, localized_unknown_command_message,
};

#[cfg(test)]
mod tests;
