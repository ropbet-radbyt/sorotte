use super::{
    localize_gui_runtime_message_legacy_compatible,
    localized_public_server_list_failed_message_legacy_compatible,
    localized_sorotte_uptodate_message_legacy_compatible,
    localized_update_check_failed_message_legacy_compatible,
};

#[test]
fn service_messages_use_selected_language() {
    assert_eq!(
        localized_sorotte_uptodate_message_legacy_compatible(Some("fr")),
        "Sorotte est a jour"
    );
    assert_eq!(
        localized_public_server_list_failed_message_legacy_compatible(Some("de")),
        "Die Liste der oeffentlichen Server konnte nicht geladen werden. Bitte besuchen Sie https://www.syncplay.pl/ in Ihrem Browser."
    );
    assert!(
        localized_update_check_failed_message_legacy_compatible(Some("fr"), "1.7.5")
            .contains("Sorotte 1.7.5")
    );
}

#[test]
fn localized_runtime_message_translates_public_server_and_update_strings() {
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Public servers refreshed: 2 entries.",
            Some("fr"),
        ),
        "Serveurs publics actualises: 2 elements."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible("Sorotte is up to date.", Some("fr")),
        "Sorotte est a jour."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "No public server refresh is currently in progress.",
            Some("fr"),
        ),
        "Aucune operation active pour \"public server refresh\"."
    );
}

#[test]
fn localized_runtime_message_preserves_english_wording_and_localizes_runtime_patterns() {
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "No public server refresh is currently in progress.",
            Some("en"),
        ),
        "No public server refresh is currently in progress."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Requesting controller access for +room:ABCDEF123456.",
            Some("fr"),
        ),
        "Demande d'acces controleur pour +room:ABCDEF123456."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible("Session reconnected.", Some("fr"),),
        "Session reconnectee."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Session state restore mismatch detected (2.500 seconds).",
            Some("fr"),
        ),
        "Ecart detecte lors de la restauration de l'etat de session (2.500 secondes)."
    );
}

#[test]
fn localized_runtime_message_translates_slowdown_and_restoration_osd_copy() {
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Slowing playback to synchronize with the room.",
            Some("fr"),
        ),
        "Ralentissement de la lecture pour synchroniser avec la salle."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Restoring normal playback speed.",
            Some("fr"),
        ),
        "Retour a la vitesse de lecture normale."
    );
    assert_eq!(
        localize_gui_runtime_message_legacy_compatible(
            "Slowing playback to synchronize with the room",
            Some("xx"),
        ),
        "Slowing playback to synchronize with the room"
    );
}
