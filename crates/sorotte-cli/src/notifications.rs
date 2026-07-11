#[cfg(test)]
use sorotte_client_app::app_boundary::notifications::{
    controller_auth_transition_notification_message as shared_controller_auth_transition_notification_message,
    format_duration_legacy as shared_format_duration_legacy,
    format_file_difference_summary as shared_format_file_difference_summary,
    localized_file_difference_summary_legacy_compatible as shared_localized_file_difference_summary_legacy_compatible,
    reconnect_transition_notification_message as shared_reconnect_transition_notification_message,
    user_change_notification_message as shared_user_change_notification_message,
};
use sorotte_client_app::app_boundary::{
    application::ClientApplication,
    diagnostics::{
        ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
        ReconnectCorrectionDiagnosticsState,
        next_reconnect_correction_diagnostic_lines_legacy_compatible as shared_next_reconnect_correction_diagnostic_lines_legacy_compatible,
    },
    notifications::{
        FileDifferenceNotificationState,
        controller_auth_notification_hidden_from_osd as shared_controller_auth_notification_hidden_from_osd,
        controller_auth_transition_notification_message_localized_legacy_compatible as shared_controller_auth_transition_notification_message_localized_legacy_compatible,
        localized_file_difference_notification_line_legacy_compatible as shared_localized_file_difference_notification_line_legacy_compatible,
        next_file_difference_notification_summary_legacy_compatible as shared_next_file_difference_notification_summary_legacy_compatible,
        reconnect_transition_notification_message_localized_legacy_compatible as shared_reconnect_transition_notification_message_localized_legacy_compatible,
        user_change_notification_hidden_from_osd as shared_user_change_notification_hidden_from_osd,
        user_change_notification_message_localized_legacy_compatible as shared_user_change_notification_message_localized_legacy_compatible,
    },
};
#[cfg(test)]
use sorotte_client_core::FileDifferenceSummary;
use sorotte_client_core::{
    AutoplayCountdownNotification, ChatNotification, ControllerAuthTransitionNotification,
    ReconnectTransitionNotification, RoomPlaystateView, UserChangeNotification,
};
use sorotte_player_api::{PlayerError, PlayerPlaybackTelemetryUpdate};
use sorotte_player_mpv::{LegacySyncplayOsdKind, MpvAdapter};

use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;

const PLAYER_DRIFT_DIAGNOSTIC_THRESHOLD_SECONDS: f64 = 1.0;

mod autoplay;
mod chat;
mod controller_auth;
mod file_difference;
mod playback_diagnostics;
mod player_osd;
mod reconnect;
mod reconnect_diagnostics;
mod user_change;

use self::player_osd::{
    emit_sorotte_player_chat_notification_legacy_compatible,
    emit_sorotte_player_osd_notification_legacy_compatible,
};

#[cfg(test)]
pub(super) use self::autoplay::{
    autoplay_countdown_notification_message_localized_legacy_compatible,
    flush_autoplay_notifications_to_sink,
};
pub(super) use self::autoplay::{
    emit_autoplay_countdown_notification, flush_autoplay_notifications_legacy_compatible,
};
pub(super) use self::chat::flush_chat_notifications_legacy_compatible;
#[cfg(test)]
pub(super) use self::chat::{chat_notification_message, flush_chat_notifications_to_sink};
pub(super) use self::controller_auth::flush_controller_auth_notifications_legacy_compatible;
#[cfg(test)]
pub(super) use self::controller_auth::{
    controller_auth_notification_hidden_from_osd, controller_auth_transition_notification_message,
    controller_auth_transition_notification_message_localized_legacy_compatible,
    flush_controller_auth_notifications_to_sink,
};
pub(super) use self::file_difference::{
    emit_file_difference_notification, flush_file_difference_notifications_legacy_compatible,
};
#[cfg(test)]
pub(super) use self::file_difference::{
    flush_file_difference_notifications_to_sink, format_file_difference_summary,
    localized_file_difference_summary_legacy_compatible,
};
pub(super) use self::playback_diagnostics::flush_player_playback_telemetry_diagnostics;
#[cfg(test)]
pub(super) use self::playback_diagnostics::{
    player_playback_drift_diagnostic_messages_localized_legacy_compatible,
    player_playback_telemetry_update_message,
    player_playback_telemetry_update_message_localized_legacy_compatible,
};
pub(super) use self::reconnect::flush_reconnect_notifications_legacy_compatible;
#[cfg(test)]
pub(super) use self::reconnect::{
    flush_reconnect_notifications_to_sink, reconnect_transition_notification_message,
    reconnect_transition_notification_message_localized_legacy_compatible,
};
pub(super) use self::reconnect_diagnostics::{
    emit_reconnect_correction_diagnostic, flush_reconnect_correction_diagnostics_to_sink,
};
pub(super) use self::user_change::flush_user_change_notifications_legacy_compatible;
#[cfg(test)]
pub(super) use self::user_change::{
    flush_user_change_notifications_to_sink, format_duration_legacy,
    user_change_notification_hidden_from_osd, user_change_notification_message,
    user_change_notification_message_localized_legacy_compatible,
};
