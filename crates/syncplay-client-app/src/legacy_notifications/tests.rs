use syncplay_client_core::{
    ControllerAuthTransitionNotification, FileDifferenceSummary, ReconnectTransitionNotification,
    UserChangeNotification,
};

use super::{
    FileDifferenceNotificationState, controller_auth_notification_hidden_from_osd,
    controller_auth_transition_notification_message,
    controller_auth_transition_notification_message_localized_legacy_compatible,
    format_duration_legacy, format_file_difference_summary,
    localized_file_difference_notification_line_legacy_compatible,
    localized_file_difference_summary_legacy_compatible,
    next_file_difference_notification_summary_legacy_compatible,
    reconnect_transition_notification_message,
    reconnect_transition_notification_message_localized_legacy_compatible,
    user_change_notification_hidden_from_osd, user_change_notification_message,
    user_change_notification_message_localized_legacy_compatible,
};

#[test]
fn reconnect_transition_notification_message_formats_legacy_strings() {
    assert_eq!(
        reconnect_transition_notification_message(&ReconnectTransitionNotification::Attempting {
            retries: 3,
            delay_seconds: 1.25,
        }),
        "Connection with server lost, attempting to reconnect (retry=3, delay_seconds=1.250)"
    );
    assert_eq!(
        reconnect_transition_notification_message_localized_legacy_compatible(
            &ReconnectTransitionNotification::Connected,
            Some("de")
        ),
        "Erneut mit dem Server verbunden"
    );
}

#[test]
fn controller_auth_notifications_format_and_honor_visibility() {
    assert_eq!(
        controller_auth_transition_notification_message(
            &ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "room-a".to_owned(),
                hide_from_osd: false,
            }
        ),
        "alice authenticated as a room operator in room room-a"
    );
    assert_eq!(
        controller_auth_transition_notification_message_localized_legacy_compatible(
            &ControllerAuthTransitionNotification::Attempting {
                room: "room-a".to_owned(),
            },
            Some("fr")
        ),
        "Identification comme operateur de salle dans la salle room-a..."
    );
    assert!(controller_auth_notification_hidden_from_osd(
        &ControllerAuthTransitionNotification::Failed {
            username: "alice".to_owned(),
            room: "room-a".to_owned(),
            hide_from_osd: true,
        }
    ));
}

#[test]
fn format_duration_legacy_matches_python_shape() {
    assert_eq!(format_duration_legacy(95.5), "01:36");
    assert_eq!(format_duration_legacy(3600.0), "01:00:00");
    assert_eq!(format_duration_legacy(604800.0), "00:00 (Title 1)");
    assert_eq!(format_duration_legacy(-1.5), "-00:02");
}

#[test]
fn user_change_notifications_format_and_honor_visibility() {
    assert_eq!(
        user_change_notification_message(&UserChangeNotification::Joined {
            username: "alice".to_owned(),
            room: "room-a".to_owned(),
            hide_from_osd: false,
        }),
        "alice has joined the room: 'room-a'"
    );
    assert_eq!(
        user_change_notification_message_localized_legacy_compatible(
            &UserChangeNotification::Left {
                username: "alice".to_owned(),
                hide_from_osd: false,
            },
            Some("de")
        ),
        "alice hat den Raum verlassen"
    );
    assert!(user_change_notification_hidden_from_osd(
        &UserChangeNotification::Playing {
            username: "alice".to_owned(),
            room: "room-a".to_owned(),
            file_name: Some("movie.mkv".to_owned()),
            file_duration: None,
            include_room_addendum: true,
            hide_from_osd: true,
        }
    ));
}

#[test]
fn file_difference_helpers_format_localize_and_dedupe() {
    assert_eq!(
        format_file_difference_summary(FileDifferenceSummary {
            filename: true,
            filesize: false,
            fileduration: true,
        }),
        Some("filename, duration".to_owned())
    );
    assert_eq!(
        localized_file_difference_summary_legacy_compatible("filename, duration", Some("de")),
        "Dateiname, Dauer"
    );
    assert_eq!(
        localized_file_difference_notification_line_legacy_compatible(
            "filename, duration",
            Some("de")
        ),
        "Dateiunterschiede: Dateiname, Dauer"
    );

    let mut state = FileDifferenceNotificationState::default();
    let first = next_file_difference_notification_summary_legacy_compatible(
        &mut state,
        Some(FileDifferenceSummary {
            filename: true,
            filesize: false,
            fileduration: false,
        }),
    );
    assert_eq!(first.as_deref(), Some("filename"));

    let second = next_file_difference_notification_summary_legacy_compatible(
        &mut state,
        Some(FileDifferenceSummary {
            filename: true,
            filesize: false,
            fileduration: false,
        }),
    );
    assert_eq!(second, None);

    let cleared = next_file_difference_notification_summary_legacy_compatible(&mut state, None);
    assert_eq!(cleared, None);
}
