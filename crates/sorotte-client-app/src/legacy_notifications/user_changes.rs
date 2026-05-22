use sorotte_client_core::UserChangeNotification;

use super::duration::format_duration_legacy;

pub fn user_change_notification_message(notification: &UserChangeNotification) -> String {
    match notification {
        UserChangeNotification::Joined { username, room, .. } => {
            format!("{username} has joined the room: '{room}'")
        }
        UserChangeNotification::Playing {
            username,
            room,
            file_name,
            file_duration,
            include_room_addendum,
            ..
        } => match file_name.as_deref() {
            Some(file_name) => {
                let mut message = if let Some(duration_seconds) = file_duration
                    .as_ref()
                    .and_then(|duration| duration.as_f64())
                {
                    format!(
                        "{username} is playing '{file_name}' ({})",
                        format_duration_legacy(duration_seconds)
                    )
                } else {
                    format!("{username} is playing '{file_name}'")
                };
                if *include_room_addendum {
                    message.push_str(&format!(" in room: '{room}'"));
                }
                message
            }
            None if *include_room_addendum => {
                format!("{username} is playing a file in room: '{room}'")
            }
            None => format!("{username} is playing a file"),
        },
        UserChangeNotification::Left { username, .. } => format!("{username} has left"),
    }
}

fn localized_user_joined_room_phrase_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "ist dem Raum beigetreten",
        Some("es") => "se ha unido a la sala",
        Some("eo") => "eniris en la cxambron",
        Some("fi") => "liittyi huoneeseen",
        Some("fr") => "a rejoint la salle",
        Some("it") => "si e unito alla stanza",
        Some("pt_PT" | "pt_BR") => "entrou na sala",
        Some("tr") => "odaya katildi",
        Some("ru") => "prisoinilsia k komnate",
        Some("zh_CN") => "jiaru le fangjian",
        Some("ko") => "bang-e chamgahasseumnida",
        _ => "has joined the room",
    }
}

fn localized_user_playing_phrase_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "spielt",
        Some("es") => "esta reproduciendo",
        Some("eo") => "ludas",
        Some("fi") => "toistaa",
        Some("fr") => "lit",
        Some("it") => "sta riproducendo",
        Some("pt_PT" | "pt_BR") => "esta reproduzindo",
        Some("tr") => "oynatiyor",
        Some("ru") => "vosproizvodit",
        Some("zh_CN") => "zhengzai bofang",
        Some("ko") => "jaesaeng jungimnida",
        _ => "is playing",
    }
}

fn localized_user_playing_file_phrase_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "spielt eine Datei",
        Some("es") => "esta reproduciendo un archivo",
        Some("eo") => "ludas dosieron",
        Some("fi") => "toistaa tiedostoa",
        Some("fr") => "lit un fichier",
        Some("it") => "sta riproducendo un file",
        Some("pt_PT" | "pt_BR") => "esta reproduzindo um arquivo",
        Some("tr") => "bir dosya oynatiyor",
        Some("ru") => "vosproizvodit fail",
        Some("zh_CN") => "zhengzai bofang wenjian",
        Some("ko") => "pail-eul jaesaeng jungimnida",
        _ => "is playing a file",
    }
}

fn localized_user_room_addendum_phrase_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "in Raum",
        Some("es") => "en la sala",
        Some("eo") => "en cxambro",
        Some("fi") => "huoneessa",
        Some("fr") => "dans la salle",
        Some("it") => "nella stanza",
        Some("pt_PT" | "pt_BR") => "na sala",
        Some("tr") => "odada",
        Some("ru") => "v komnate",
        Some("zh_CN") => "zai fangjian",
        Some("ko") => "bangeseo",
        _ => "in room",
    }
}

fn localized_user_left_phrase_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "hat den Raum verlassen",
        Some("es") => "ha salido",
        Some("eo") => "foriris",
        Some("fi") => "poistui",
        Some("fr") => "a quitte la salle",
        Some("it") => "ha lasciato la stanza",
        Some("pt_PT" | "pt_BR") => "saiu",
        Some("tr") => "ayrildi",
        Some("ru") => "pokinul komnatu",
        Some("zh_CN") => "likai le",
        Some("ko") => "bang-eul nagasseumnida",
        _ => "has left",
    }
}

pub fn user_change_notification_message_localized_legacy_compatible(
    notification: &UserChangeNotification,
    language: Option<&str>,
) -> String {
    match notification {
        UserChangeNotification::Joined { username, room, .. } => format!(
            "{username} {}: '{room}'",
            localized_user_joined_room_phrase_legacy_compatible(language)
        ),
        UserChangeNotification::Playing {
            username,
            room,
            file_name,
            file_duration,
            include_room_addendum,
            ..
        } => match file_name.as_deref() {
            Some(file_name) => {
                let mut message = if let Some(duration_seconds) = file_duration
                    .as_ref()
                    .and_then(|duration| duration.as_f64())
                {
                    format!(
                        "{username} {} '{file_name}' ({})",
                        localized_user_playing_phrase_legacy_compatible(language),
                        format_duration_legacy(duration_seconds)
                    )
                } else {
                    format!(
                        "{username} {} '{file_name}'",
                        localized_user_playing_phrase_legacy_compatible(language)
                    )
                };
                if *include_room_addendum {
                    message.push_str(&format!(
                        " {}: '{room}'",
                        localized_user_room_addendum_phrase_legacy_compatible(language)
                    ));
                }
                message
            }
            None if *include_room_addendum => format!(
                "{username} {} {}: '{room}'",
                localized_user_playing_file_phrase_legacy_compatible(language),
                localized_user_room_addendum_phrase_legacy_compatible(language)
            ),
            None => format!(
                "{username} {}",
                localized_user_playing_file_phrase_legacy_compatible(language)
            ),
        },
        UserChangeNotification::Left { username, .. } => format!(
            "{username} {}",
            localized_user_left_phrase_legacy_compatible(language)
        ),
    }
}

pub fn user_change_notification_hidden_from_osd(notification: &UserChangeNotification) -> bool {
    match notification {
        UserChangeNotification::Joined { hide_from_osd, .. }
        | UserChangeNotification::Playing { hide_from_osd, .. }
        | UserChangeNotification::Left { hide_from_osd, .. } => *hide_from_osd,
    }
}
