mod controller_auth;
mod duration;
mod file_differences;
mod reconnect;
mod user_changes;

#[cfg(test)]
mod tests;

pub use controller_auth::{
    controller_auth_notification_hidden_from_osd, controller_auth_transition_notification_message,
    controller_auth_transition_notification_message_localized_legacy_compatible,
};
pub use duration::format_duration_legacy;
pub use file_differences::{
    FileDifferenceNotificationState, format_file_difference_summary,
    localized_file_difference_notification_line_legacy_compatible,
    localized_file_difference_summary_legacy_compatible,
    localized_file_differences_prefix_legacy_compatible,
    next_file_difference_notification_summary_legacy_compatible,
};
pub use reconnect::{
    reconnect_transition_notification_message,
    reconnect_transition_notification_message_localized_legacy_compatible,
};
pub use user_changes::{
    user_change_notification_hidden_from_osd, user_change_notification_message,
    user_change_notification_message_localized_legacy_compatible,
};
