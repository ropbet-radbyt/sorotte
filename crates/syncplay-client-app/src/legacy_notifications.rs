use syncplay_client_core::{
    ControllerAuthTransitionNotification, FileDifferenceSummary, ReconnectTransitionNotification,
    UserChangeNotification,
};

const ROUND_HALF_EPSILON: f64 = 1e-12;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileDifferenceNotificationState {
    last_summary: Option<String>,
}

pub fn reconnect_transition_notification_message(
    notification: &ReconnectTransitionNotification,
) -> String {
    match notification {
        ReconnectTransitionNotification::Attempting {
            retries,
            delay_seconds,
        } => format!(
            "Connection with server lost, attempting to reconnect (retry={retries}, delay_seconds={delay_seconds:.3})"
        ),
        ReconnectTransitionNotification::Connected => "Reconnected to server".to_owned(),
        ReconnectTransitionNotification::Disconnected => {
            "Connection with server lost, reconnect attempts exhausted".to_owned()
        }
        ReconnectTransitionNotification::RestoringState => {
            "Restoring local state after reconnect...".to_owned()
        }
        ReconnectTransitionNotification::StateRestoreValidationMismatch {
            local_paused,
            room_paused,
            local_position,
            room_position,
            position_diff_seconds,
        } => format!(
            "Reconnect state restore validation mismatch; correcting local player: player(paused={local_paused}, position={local_position:.3}) room(paused={room_paused}, position={room_position:.3}) diff={position_diff_seconds:.3}"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
            attempt,
            max_attempts,
            cooldown_ticks,
        } => format!(
            "Reconnect state restore correction failed; scheduling retry (attempt={attempt}/{max_attempts}, cooldown_ticks={cooldown_ticks})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
            attempts,
            max_attempts,
        } => format!(
            "Reconnect state restore correction failed; retry budget exhausted (attempts={attempts}, max_attempts={max_attempts}), stopping auto-correction for this restore cycle"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
            consecutive_mismatch_cycles,
            disable_after_mismatch_cycles,
        } => format!(
            "Reconnect state restore correction disabled after repeated mismatches (consecutive_mismatch_cycles={consecutive_mismatch_cycles}, threshold={disable_after_mismatch_cycles})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
            remaining_reconnect_cycles_after_this_cycle,
        } => format!(
            "Reconnect state restore correction suppressed for recovery cooldown (remaining_reconnect_cycles_after_this_cycle={remaining_reconnect_cycles_after_this_cycle})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled => {
            "Reconnect state restore correction re-enabled after recovery cooldown".to_owned()
        }
        ReconnectTransitionNotification::RestoringPlaylist => {
            "Restoring playlist on reconnect...".to_owned()
        }
    }
}

pub fn reconnect_transition_notification_message_localized_legacy_compatible(
    notification: &ReconnectTransitionNotification,
    language: Option<&str>,
) -> String {
    match notification {
        ReconnectTransitionNotification::Attempting {
            retries,
            delay_seconds,
        } => match language {
            Some("de") => format!(
                "Verbindung zum Server verloren, erneuter Verbindungsversuch (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("es") => format!(
                "Conexion con el servidor perdida, intentando reconectar (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("eo") => format!(
                "Konekto al servilo perdita, provas rekonekti (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("fi") => format!(
                "Yhteys palvelimeen katkesi, yritetaan yhdistaa uudelleen (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("fr") => format!(
                "Connexion au serveur perdue, tentative de reconnexion (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("it") => format!(
                "Connessione al server persa, tentativo di riconnessione (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("pt_PT" | "pt_BR") => format!(
                "Conexao com o servidor perdida, tentando reconectar (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("tr") => format!(
                "Sunucu baglantisi kesildi, yeniden baglanmaya calisiliyor (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("ru") => format!(
                "Soedinenie s serverom poterianno, popytka povtornogo podkliucheniia (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("zh_CN") => format!(
                "Yu fuwuqi de lianjie yi diu shi, zhengzai changshi chongxin lianjie (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("ko") => format!(
                "Seobeowa-ui yeongyeori kkeun-eojyeosseum, dasi yeongyeol si-do jung (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::Connected => match language {
            Some("de") => "Erneut mit dem Server verbunden".to_owned(),
            Some("es") => "Reconectado al servidor".to_owned(),
            Some("eo") => "Rekonektita al servilo".to_owned(),
            Some("fi") => "Yhdistetty uudelleen palvelimeen".to_owned(),
            Some("fr") => "Reconnecte au serveur".to_owned(),
            Some("it") => "Riconnesso al server".to_owned(),
            Some("pt_PT" | "pt_BR") => "Reconectado ao servidor".to_owned(),
            Some("tr") => "Sunucuya yeniden baglanildi".to_owned(),
            Some("ru") => "Povtornoe podkliuchenie k serveru vypolneno".to_owned(),
            Some("zh_CN") => "Yi chongxin lianjie dao fuwuqi".to_owned(),
            Some("ko") => "Seobeoe dasi yeongyeoldoeeotseumnida".to_owned(),
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::Disconnected => match language {
            Some("de") => {
                "Verbindung zum Server verloren, Wiederverbindungsversuche erschoepft"
                    .to_owned()
            }
            Some("es") => {
                "Conexion con el servidor perdida, intentos de reconexion agotados".to_owned()
            }
            Some("eo") => {
                "Konekto al servilo perdita, rekonektaj provoj eluzitaj".to_owned()
            }
            Some("fi") => {
                "Yhteys palvelimeen katkesi, uudelleenyhdistamisyritykset loppuivat"
                    .to_owned()
            }
            Some("fr") => {
                "Connexion au serveur perdue, tentatives de reconnexion epuisees".to_owned()
            }
            Some("it") => {
                "Connessione al server persa, tentativi di riconnessione esauriti".to_owned()
            }
            Some("pt_PT" | "pt_BR") => {
                "Conexao com o servidor perdida, tentativas de reconexao esgotadas".to_owned()
            }
            Some("tr") => {
                "Sunucu baglantisi kesildi, yeniden baglanma denemeleri tukendi"
                    .to_owned()
            }
            Some("ru") => {
                "Soedinenie s serverom poterianno, popytki povtornogo podkliucheniia ischerpany"
                    .to_owned()
            }
            Some("zh_CN") => {
                "Yu fuwuqi de lianjie yi diu shi, chongxin lianjie changshi yongjin".to_owned()
            }
            Some("ko") => {
                "Seobeowa-ui yeongyeori kkeun-eojyeosseum, dasi yeongyeol si-do-reul modu sayonghaetseumnida"
                    .to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::RestoringState => match language {
            Some("de") => "Lokalen Status nach Wiederverbindung wiederherstellen...".to_owned(),
            Some("es") => "Restaurando estado local tras la reconexion...".to_owned(),
            Some("eo") => "Restarigante lokan staton post rekonekto...".to_owned(),
            Some("fi") => "Palautetaan paikallinen tila uudelleenyhdistamisen jalkeen..."
                .to_owned(),
            Some("fr") => "Restauration de l'etat local apres reconnexion...".to_owned(),
            Some("it") => "Ripristino dello stato locale dopo la riconnessione...".to_owned(),
            Some("pt_PT" | "pt_BR") => {
                "Restaurando estado local apos a reconexao...".to_owned()
            }
            Some("tr") => "Yeniden baglanti sonrasi yerel durum geri yukleniyor...".to_owned(),
            Some("ru") => {
                "Vosstanovlenie lokalnogo sostoianiia posle povtornogo podkliucheniia..."
                    .to_owned()
            }
            Some("zh_CN") => "Zhengzai zai chongxin lianjie hou huifu bendi zhuangtai..."
                .to_owned(),
            Some("ko") => {
                "Dasi yeongyeol hu lokal sangtaereul bokguhaneun jung...".to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::RestoringPlaylist => match language {
            Some("de") => "Playlist nach Wiederverbindung wiederherstellen...".to_owned(),
            Some("es") => "Restaurando lista de reproduccion tras la reconexion...".to_owned(),
            Some("eo") => "Restarigante ludliston post rekonekto...".to_owned(),
            Some("fi") => {
                "Palautetaan soittolista uudelleenyhdistamisen jalkeen...".to_owned()
            }
            Some("fr") => "Restauration de la liste de lecture apres reconnexion...".to_owned(),
            Some("it") => "Ripristino della playlist dopo la riconnessione...".to_owned(),
            Some("pt_PT" | "pt_BR") => {
                "Restaurando lista de reproducao apos a reconexao...".to_owned()
            }
            Some("tr") => {
                "Yeniden baglanti sonrasi oynatma listesi geri yukleniyor...".to_owned()
            }
            Some("ru") => {
                "Vosstanovlenie spiska vosproizvedeniia posle povtornogo podkliucheniia..."
                    .to_owned()
            }
            Some("zh_CN") => "Zhengzai zai chongxin lianjie hou huifu bofang liebiao..."
                .to_owned(),
            Some("ko") => {
                "Dasi yeongyeol hu jaesaeng mongnog-eul bokguhaneun jung...".to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        _ => reconnect_transition_notification_message(notification),
    }
}

pub fn controller_auth_transition_notification_message(
    notification: &ControllerAuthTransitionNotification,
) -> String {
    match notification {
        ControllerAuthTransitionNotification::Attempting { room } => {
            format!("Identifying as room operator in room {room}...")
        }
        ControllerAuthTransitionNotification::Succeeded { username, room, .. } => {
            format!("{username} authenticated as a room operator in room {room}")
        }
        ControllerAuthTransitionNotification::Failed { username, room, .. } => {
            format!("{username} failed to identify as a room operator in room {room}")
        }
    }
}

pub fn controller_auth_transition_notification_message_localized_legacy_compatible(
    notification: &ControllerAuthTransitionNotification,
    language: Option<&str>,
) -> String {
    match notification {
        ControllerAuthTransitionNotification::Attempting { room } => match language {
            Some("de") => format!("Identifiziere als Raumoperator in Raum {room}..."),
            Some("es") => {
                format!("Identificandose como operador de la sala en la sala {room}...")
            }
            Some("eo") => {
                format!("Identigante kiel cxambro-operatoro en cxambro {room}...")
            }
            Some("fi") => {
                format!("Tunnistaudutaan huoneen operaattoriksi huoneessa {room}...")
            }
            Some("fr") => {
                format!("Identification comme operateur de salle dans la salle {room}...")
            }
            Some("it") => {
                format!("Identificazione come operatore della stanza nella stanza {room}...")
            }
            Some("pt_PT" | "pt_BR") => {
                format!("Identificando como operador da sala na sala {room}...")
            }
            Some("tr") => {
                format!("Oda operatoru olarak {room} odasinda kimlik dogrulaniyor...")
            }
            Some("ru") => format!("Identifikatsiia kak operator komnaty v komnate {room}..."),
            Some("zh_CN") => {
                format!("Zhengzai zai fangjian {room} zhong yanzheng wei fangjian guanliyuan...")
            }
            Some("ko") => {
                format!("Bang {room}eseo bang unyeongja-ro inyong jung...")
            }
            _ => controller_auth_transition_notification_message(notification),
        },
        ControllerAuthTransitionNotification::Succeeded { username, room, .. } => match language {
            Some("de") => {
                format!("{username} wurde als Raumoperator in Raum {room} authentifiziert")
            }
            Some("es") => {
                format!("{username} se autentico como operador de la sala en la sala {room}")
            }
            Some("eo") => {
                format!("{username} sukcese identigxis kiel cxambro-operatoro en cxambro {room}")
            }
            Some("fi") => {
                format!("{username} tunnistautui huoneen operaattoriksi huoneessa {room}")
            }
            Some("fr") => {
                format!(
                    "{username} s'est authentifie comme operateur de salle dans la salle {room}"
                )
            }
            Some("it") => {
                format!(
                    "{username} si e autenticato come operatore della stanza nella stanza {room}"
                )
            }
            Some("pt_PT" | "pt_BR") => {
                format!("{username} foi autenticado como operador da sala na sala {room}")
            }
            Some("tr") => {
                format!("{username}, {room} odasinda oda operatoru olarak dogrulandi")
            }
            Some("ru") => {
                format!("{username} identifitsirovan kak operator komnaty v komnate {room}")
            }
            Some("zh_CN") => {
                format!("{username} yi zai fangjian {room} zhong yanzheng wei fangjian guanliyuan")
            }
            Some("ko") => {
                format!("{username}neun bang {room}eseo bang unyeongja-ro inyongdoeeotseumnida")
            }
            _ => controller_auth_transition_notification_message(notification),
        },
        ControllerAuthTransitionNotification::Failed { username, room, .. } => match language {
            Some("de") => format!(
                "{username} konnte sich nicht als Raumoperator in Raum {room} identifizieren"
            ),
            Some("es") => format!(
                "{username} no pudo identificarse como operador de la sala en la sala {room}"
            ),
            Some("eo") => {
                format!("{username} malsukcesis identigxi kiel cxambro-operatoro en cxambro {room}")
            }
            Some("fi") => format!(
                "{username} epaonnistui tunnistautumaan huoneen operaattoriksi huoneessa {room}"
            ),
            Some("fr") => format!(
                "{username} n'a pas pu s'identifier comme operateur de salle dans la salle {room}"
            ),
            Some("it") => format!(
                "{username} non e riuscito a identificarsi come operatore della stanza nella stanza {room}"
            ),
            Some("pt_PT" | "pt_BR") => format!(
                "{username} nao conseguiu se identificar como operador da sala na sala {room}"
            ),
            Some("tr") => {
                format!("{username}, {room} odasinda oda operatoru olarak kimligini dogrulayamadi")
            }
            Some("ru") => format!(
                "{username} ne smog identifitsirovatsia kak operator komnaty v komnate {room}"
            ),
            Some("zh_CN") => format!(
                "{username} wei neng zai fangjian {room} zhong yanzheng wei fangjian guanliyuan"
            ),
            Some("ko") => format!(
                "{username}neun bang {room}eseo bang unyeongja-ro inyonghaji moshaetseumnida"
            ),
            _ => controller_auth_transition_notification_message(notification),
        },
    }
}

pub fn controller_auth_notification_hidden_from_osd(
    notification: &ControllerAuthTransitionNotification,
) -> bool {
    match notification {
        ControllerAuthTransitionNotification::Attempting { .. } => false,
        ControllerAuthTransitionNotification::Succeeded { hide_from_osd, .. }
        | ControllerAuthTransitionNotification::Failed { hide_from_osd, .. } => *hide_from_osd,
    }
}

fn round_half_to_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;

    if fraction + ROUND_HALF_EPSILON < 0.5 {
        return floor;
    }
    if fraction - ROUND_HALF_EPSILON > 0.5 {
        return floor + 1.0;
    }

    if floor.rem_euclid(2.0) == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

pub fn format_duration_legacy(time_seconds: f64) -> String {
    let sign = if time_seconds < 0.0 { "-" } else { "" };
    let rounded_seconds = round_half_to_even(time_seconds.abs()) as u64;

    let mut weeks = rounded_seconds / 604_800;
    let title = if weeks > 0 {
        let title = weeks;
        weeks = 0;
        title
    } else {
        0
    };
    let days = (rounded_seconds % 604_800) / 86_400;
    let hours = (rounded_seconds % 86_400) / 3_600;
    let minutes = (rounded_seconds % 3_600) / 60;
    let seconds = rounded_seconds % 60;

    let mut formatted = if weeks > 0 {
        format!("{sign}{weeks}w, {days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if days > 0 {
        format!("{sign}{days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{minutes:02}:{seconds:02}")
    };

    if title > 0 {
        formatted.push_str(&format!(" (Title {title})"));
    }

    formatted
}

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

pub fn format_file_difference_summary(summary: FileDifferenceSummary) -> Option<String> {
    let mut differences = Vec::new();
    if summary.filename {
        differences.push("filename");
    }
    if summary.filesize {
        differences.push("filesize");
    }
    if summary.fileduration {
        differences.push("duration");
    }

    if differences.is_empty() {
        None
    } else {
        Some(differences.join(", "))
    }
}

pub fn localized_file_differences_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Dateiunterschiede",
        Some("es") => "Diferencias de archivo",
        Some("eo") => "Dosieraj diferencoj",
        Some("fi") => "Tiedostoerot",
        Some("fr") => "Differences de fichier",
        Some("it") => "Differenze del file",
        Some("pt_PT" | "pt_BR") => "Diferencas de arquivo",
        Some("tr") => "Dosya farklari",
        Some("ru") => "Razlichiia failov",
        Some("zh_CN") => "Wenjian chayi",
        Some("ko") => "Pail chai",
        _ => "file differences",
    }
}

fn localized_file_difference_token_legacy_compatible(
    token: &str,
    language: Option<&str>,
) -> String {
    match (token, language) {
        ("filename", Some("de")) => "Dateiname".to_owned(),
        ("filename", Some("es")) => "nombre de archivo".to_owned(),
        ("filename", Some("eo")) => "dosiernomo".to_owned(),
        ("filename", Some("fi")) => "tiedostonimi".to_owned(),
        ("filename", Some("fr")) => "nom du fichier".to_owned(),
        ("filename", Some("it")) => "nome file".to_owned(),
        ("filename", Some("pt_PT" | "pt_BR")) => "nome do arquivo".to_owned(),
        ("filename", Some("tr")) => "dosya adi".to_owned(),
        ("filename", Some("ru")) => "imia faila".to_owned(),
        ("filename", Some("zh_CN")) => "wenjian mingcheng".to_owned(),
        ("filename", Some("ko")) => "pail ireum".to_owned(),
        ("filesize", Some("de")) => "Dateigroesse".to_owned(),
        ("filesize", Some("es")) => "tamano del archivo".to_owned(),
        ("filesize", Some("eo")) => "dosiergrando".to_owned(),
        ("filesize", Some("fi")) => "tiedostokoko".to_owned(),
        ("filesize", Some("fr")) => "taille du fichier".to_owned(),
        ("filesize", Some("it")) => "dimensione del file".to_owned(),
        ("filesize", Some("pt_PT" | "pt_BR")) => "tamanho do arquivo".to_owned(),
        ("filesize", Some("tr")) => "dosya boyutu".to_owned(),
        ("filesize", Some("ru")) => "razmer faila".to_owned(),
        ("filesize", Some("zh_CN")) => "wenjian daxiao".to_owned(),
        ("filesize", Some("ko")) => "pail keugi".to_owned(),
        ("duration", Some("de")) => "Dauer".to_owned(),
        ("duration", Some("es")) => "duracion".to_owned(),
        ("duration", Some("eo")) => "dauro".to_owned(),
        ("duration", Some("fi")) => "kesto".to_owned(),
        ("duration", Some("fr")) => "duree".to_owned(),
        ("duration", Some("it")) => "durata".to_owned(),
        ("duration", Some("pt_PT" | "pt_BR")) => "duracao".to_owned(),
        ("duration", Some("tr")) => "sure".to_owned(),
        ("duration", Some("ru")) => "dlitelnost".to_owned(),
        ("duration", Some("zh_CN")) => "shichang".to_owned(),
        ("duration", Some("ko")) => "gigan".to_owned(),
        _ => match token {
            "filename" => "filename".to_owned(),
            "filesize" => "filesize".to_owned(),
            "duration" => "duration".to_owned(),
            _ => token.to_owned(),
        },
    }
}

pub fn localized_file_difference_summary_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    summary
        .split(", ")
        .map(|token| localized_file_difference_token_legacy_compatible(token, language))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn localized_file_difference_notification_line_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    format!(
        "{}: {}",
        localized_file_differences_prefix_legacy_compatible(language),
        localized_file_difference_summary_legacy_compatible(summary, language)
    )
}

pub fn next_file_difference_notification_summary_legacy_compatible(
    state: &mut FileDifferenceNotificationState,
    summary: Option<FileDifferenceSummary>,
) -> Option<String> {
    let summary = summary.and_then(format_file_difference_summary);

    match summary {
        Some(summary) => {
            if state.last_summary.as_deref() != Some(summary.as_str()) {
                state.last_summary = Some(summary.clone());
                Some(summary)
            } else {
                state.last_summary = Some(summary);
                None
            }
        }
        None => {
            state.last_summary = None;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use syncplay_client_core::{
        ControllerAuthTransitionNotification, FileDifferenceSummary,
        ReconnectTransitionNotification, UserChangeNotification,
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
            reconnect_transition_notification_message(
                &ReconnectTransitionNotification::Attempting {
                    retries: 3,
                    delay_seconds: 1.25,
                }
            ),
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
}
